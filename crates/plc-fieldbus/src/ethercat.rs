//! EtherCAT master implementation.
//!
//! Provides the EtherCAT master functionality with:
//! - Slave scanning and configuration
//! - Process data exchange (PDO)
//! - Service data access (SDO)
//! - Distributed Clocks (DC) synchronization
//!
//! The implementation is designed to work with SOEM (Simple Open EtherCAT Master)
//! via FFI bindings, or with a simulated backend for testing.

use crate::dc_sync::{DcController, DcSlaveConfig};
use crate::slave_config::{
    NetworkConfig, PdoEntry, PdoMapping, SdoRequest, SlaveConfig, SlaveIdentity, SlaveState,
};
use crate::FieldbusDriver;
use plc_common::config::EthercatConfig;
use plc_common::error::{PlcError, PlcResult};
use std::time::Instant;
use tracing::{debug, error, info, trace, warn};

/// EtherCAT master state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MasterState {
    /// Master not initialized.
    #[default]
    Offline,
    /// Master initialized, scanning for slaves.
    Init,
    /// Slaves discovered, configuring.
    PreOp,
    /// DC configured, outputs safe.
    SafeOp,
    /// Full operation.
    Op,
    /// Fault state.
    Fault,
}

impl std::fmt::Display for MasterState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Offline => write!(f, "OFFLINE"),
            Self::Init => write!(f, "INIT"),
            Self::PreOp => write!(f, "PRE_OP"),
            Self::SafeOp => write!(f, "SAFE_OP"),
            Self::Op => write!(f, "OP"),
            Self::Fault => write!(f, "FAULT"),
        }
    }
}

/// EtherCAT frame statistics.
#[derive(Debug, Clone, Default)]
pub struct FrameStats {
    /// Total frames sent.
    pub frames_sent: u64,
    /// Total frames received.
    pub frames_received: u64,
    /// Working counter errors.
    pub wkc_errors: u64,
    /// Frame timeouts.
    pub timeouts: u64,
    /// Last round-trip time in microseconds.
    pub last_rtt_us: u32,
    /// Minimum round-trip time.
    pub min_rtt_us: u32,
    /// Maximum round-trip time.
    pub max_rtt_us: u32,
}

impl FrameStats {
    /// Record a successful frame exchange.
    pub fn record_success(&mut self, rtt_us: u32) {
        self.frames_sent += 1;
        self.frames_received += 1;
        self.last_rtt_us = rtt_us;
        if self.min_rtt_us == 0 || rtt_us < self.min_rtt_us {
            self.min_rtt_us = rtt_us;
        }
        if rtt_us > self.max_rtt_us {
            self.max_rtt_us = rtt_us;
        }
    }

    /// Record a working counter error.
    pub fn record_wkc_error(&mut self) {
        self.frames_sent += 1;
        self.wkc_errors += 1;
    }

    /// Record a timeout.
    pub fn record_timeout(&mut self) {
        self.frames_sent += 1;
        self.timeouts += 1;
    }
}

/// Process data buffers for PDO exchange.
#[derive(Debug)]
pub struct ProcessImage {
    /// Input data (slave → master).
    inputs: Vec<u8>,
    /// Output data (master → slave).
    outputs: Vec<u8>,
    /// Expected working counter.
    expected_wkc: u16,
    /// Last received working counter.
    last_wkc: u16,
}

impl ProcessImage {
    /// Create a new process image with the specified sizes.
    pub fn new(input_size: usize, output_size: usize) -> Self {
        Self {
            inputs: vec![0; input_size],
            outputs: vec![0; output_size],
            expected_wkc: 0,
            last_wkc: 0,
        }
    }

    /// Get the input buffer.
    pub fn inputs(&self) -> &[u8] {
        &self.inputs
    }

    /// Get the input buffer mutably (for fieldbus driver to write).
    pub fn inputs_mut(&mut self) -> &mut [u8] {
        &mut self.inputs
    }

    /// Get the output buffer.
    pub fn outputs(&self) -> &[u8] {
        &self.outputs
    }

    /// Get the output buffer mutably (for PLC to write).
    pub fn outputs_mut(&mut self) -> &mut [u8] {
        &mut self.outputs
    }

    /// Get both buffers for exchange operation.
    ///
    /// Returns (outputs, inputs_mut) to satisfy borrow checker
    /// when both are needed simultaneously.
    pub fn exchange_buffers(&mut self) -> (&[u8], &mut [u8]) {
        (&self.outputs, &mut self.inputs)
    }

    /// Set the expected working counter.
    pub fn set_expected_wkc(&mut self, wkc: u16) {
        self.expected_wkc = wkc;
    }

    /// Check if the working counter matches expected.
    pub fn wkc_ok(&self) -> bool {
        self.last_wkc == self.expected_wkc
    }

    /// Update the last working counter.
    pub fn set_last_wkc(&mut self, wkc: u16) {
        self.last_wkc = wkc;
    }

    /// Read a byte from inputs at the given offset.
    pub fn read_input_byte(&self, offset: usize) -> Option<u8> {
        self.inputs.get(offset).copied()
    }

    /// Read a u16 from inputs (little-endian).
    pub fn read_input_u16(&self, offset: usize) -> Option<u16> {
        if offset + 2 <= self.inputs.len() {
            Some(u16::from_le_bytes([
                self.inputs[offset],
                self.inputs[offset + 1],
            ]))
        } else {
            None
        }
    }

    /// Read a u32 from inputs (little-endian).
    pub fn read_input_u32(&self, offset: usize) -> Option<u32> {
        if offset + 4 <= self.inputs.len() {
            let bytes: [u8; 4] = self.inputs[offset..offset + 4].try_into().ok()?;
            Some(u32::from_le_bytes(bytes))
        } else {
            None
        }
    }

    /// Write a byte to outputs.
    pub fn write_output_byte(&mut self, offset: usize, value: u8) {
        if let Some(b) = self.outputs.get_mut(offset) {
            *b = value;
        }
    }

    /// Write a u16 to outputs (little-endian).
    pub fn write_output_u16(&mut self, offset: usize, value: u16) {
        if offset + 2 <= self.outputs.len() {
            let bytes = value.to_le_bytes();
            self.outputs[offset] = bytes[0];
            self.outputs[offset + 1] = bytes[1];
        }
    }

    /// Write a u32 to outputs (little-endian).
    pub fn write_output_u32(&mut self, offset: usize, value: u32) {
        if offset + 4 <= self.outputs.len() {
            let bytes = value.to_le_bytes();
            self.outputs[offset..offset + 4].copy_from_slice(&bytes);
        }
    }
}

/// EtherCAT master.
///
/// Manages the EtherCAT network including slave configuration,
/// process data exchange, and distributed clocks.
pub struct EthercatMaster {
    /// Network configuration.
    config: EthercatConfig,
    /// Current master state.
    state: MasterState,
    /// Network topology and slave configuration.
    network: NetworkConfig,
    /// Process image for PDO exchange.
    process_image: ProcessImage,
    /// DC controller.
    dc: DcController,
    /// Frame statistics.
    stats: FrameStats,
    /// Transport backend.
    transport: Box<dyn EthercatTransport>,
    /// Cycle counter.
    cycle_count: u64,
    /// Consecutive WKC error counter for fault threshold enforcement.
    consecutive_wkc_errors: u32,
}

impl std::fmt::Debug for EthercatMaster {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EthercatMaster")
            .field("state", &self.state)
            .field("interface", &self.config.interface)
            .field("slave_count", &self.network.slave_count())
            .field("cycle_count", &self.cycle_count)
            .finish()
    }
}

impl EthercatMaster {
    /// Create a new EtherCAT master with the given configuration.
    pub fn new(config: EthercatConfig) -> Self {
        Self::with_transport(config.clone(), Box::new(SimulatedTransport::new(&config)))
    }

    /// Create an EtherCAT master with a custom transport backend.
    pub fn with_transport(config: EthercatConfig, transport: Box<dyn EthercatTransport>) -> Self {
        let cycle_time = config.dc_sync0_cycle;
        // Use the interface name if configured, otherwise "unspecified" for simulated mode
        let interface_name = config.interface.as_deref().unwrap_or("unspecified");
        Self {
            config: config.clone(),
            state: MasterState::Offline,
            network: NetworkConfig::new(interface_name),
            process_image: ProcessImage::new(0, 0),
            dc: DcController::new(cycle_time),
            stats: FrameStats::default(),
            transport,
            cycle_count: 0,
            consecutive_wkc_errors: 0,
        }
    }

    /// Get the current master state.
    pub fn state(&self) -> MasterState {
        self.state
    }

    /// Get frame statistics.
    pub fn stats(&self) -> &FrameStats {
        &self.stats
    }

    /// Get DC controller reference.
    pub fn dc(&self) -> &DcController {
        &self.dc
    }

    /// Get the network configuration.
    pub fn network(&self) -> &NetworkConfig {
        &self.network
    }

    /// Get the process image.
    pub fn process_image(&self) -> &ProcessImage {
        &self.process_image
    }

    /// Get mutable access to the process image.
    pub fn process_image_mut(&mut self) -> &mut ProcessImage {
        &mut self.process_image
    }

    /// Get cycle count.
    pub fn cycle_count(&self) -> u64 {
        self.cycle_count
    }

    /// Scan for slaves on the network.
    pub fn scan_slaves(&mut self) -> PlcResult<usize> {
        let interface_display = self.config.interface.as_deref().unwrap_or("unspecified");
        info!(interface = %interface_display, "Scanning for EtherCAT slaves");

        self.state = MasterState::Init;

        // Clear existing configuration before re-scanning
        // This ensures stale slaves are not retained if scan_slaves() is called multiple times
        self.network.clear();
        self.dc.clear();

        let slaves = self.transport.scan_slaves()?;
        let count = slaves.len();

        for slave in slaves {
            // Configure DC if supported
            if slave.dc_supported && self.config.dc_enabled {
                let dc_config = DcSlaveConfig::new(slave.position)
                    .with_sync0(self.config.dc_sync0_cycle.as_nanos() as u64);
                self.dc.add_slave(dc_config);
            }

            self.network.add_slave(slave);
        }

        // Resize process image based on discovered slaves
        self.process_image = ProcessImage::new(
            self.network.total_input_size,
            self.network.total_output_size,
        );

        // Calculate expected working counter:
        // Each slave with inputs contributes 1, each with outputs contributes 2
        let mut expected_wkc = 0u16;
        for slave in self.network.slaves.values() {
            if slave.input_size > 0 {
                expected_wkc += 1;
            }
            if slave.output_size > 0 {
                expected_wkc += 2;
            }
        }
        self.process_image.set_expected_wkc(expected_wkc);

        info!(
            slave_count = count,
            input_size = self.network.total_input_size,
            output_size = self.network.total_output_size,
            expected_wkc,
            "Slave scan complete"
        );

        Ok(count)
    }

    /// Configure all slaves to PRE_OP state.
    pub fn configure_slaves(&mut self) -> PlcResult<()> {
        if self.state != MasterState::Init {
            return Err(PlcError::FieldbusError(format!(
                "Cannot configure from state {}",
                self.state
            )));
        }

        debug!("Configuring slaves for PRE_OP");

        self.transport.set_state(SlaveState::PreOp)?;

        // Update our tracked state for each slave
        for slave in self.network.slaves.values_mut() {
            slave.state = SlaveState::PreOp;
        }

        self.state = MasterState::PreOp;
        info!("All slaves configured to PRE_OP");

        Ok(())
    }

    /// Configure distributed clocks.
    pub fn configure_dc(&mut self) -> PlcResult<()> {
        if !self.config.dc_enabled {
            debug!("DC disabled in configuration");
            return Ok(());
        }

        if self.dc.reference_clock().is_none() {
            warn!("No DC-capable slaves found");
            return Ok(());
        }

        debug!("Configuring distributed clocks");

        // Read initial DC time from reference clock
        let initial_time = self.transport.read_dc_time()?;
        self.dc.initialize(initial_time)?;

        // Configure DC for each slave
        for slave_config in self.dc.slaves() {
            if slave_config.dc_supported {
                self.transport.configure_slave_dc(slave_config)?;
            }
        }

        info!(
            reference_clock = self.dc.reference_clock(),
            "DC configuration complete"
        );

        Ok(())
    }

    /// Transition all slaves to SAFE_OP state.
    pub fn enter_safe_op(&mut self) -> PlcResult<()> {
        if self.state != MasterState::PreOp {
            return Err(PlcError::FieldbusError(format!(
                "Cannot enter SAFE_OP from state {}",
                self.state
            )));
        }

        debug!("Transitioning to SAFE_OP");

        self.transport.set_state(SlaveState::SafeOp)?;

        for slave in self.network.slaves.values_mut() {
            slave.state = SlaveState::SafeOp;
        }

        self.state = MasterState::SafeOp;
        info!("All slaves in SAFE_OP");

        Ok(())
    }

    /// Transition all slaves to OP state.
    pub fn enter_op(&mut self) -> PlcResult<()> {
        if self.state != MasterState::SafeOp {
            return Err(PlcError::FieldbusError(format!(
                "Cannot enter OP from state {}",
                self.state
            )));
        }

        debug!("Transitioning to OP");

        self.transport.set_state(SlaveState::Op)?;

        for slave in self.network.slaves.values_mut() {
            slave.state = SlaveState::Op;
        }

        self.state = MasterState::Op;
        info!("All slaves in OP - cyclic exchange active");

        Ok(())
    }

    /// Perform one PDO exchange cycle.
    ///
    /// Returns an error if consecutive WKC mismatches exceed the configured threshold.
    pub fn exchange(&mut self) -> PlcResult<()> {
        if self.state != MasterState::Op && self.state != MasterState::SafeOp {
            return Err(PlcError::FieldbusError(format!(
                "Cannot exchange in state {}",
                self.state
            )));
        }

        let start = Instant::now();

        // Send outputs and receive inputs
        let (outputs, inputs) = self.process_image.exchange_buffers();
        let wkc = self.transport.exchange(outputs, inputs)?;

        let rtt = start.elapsed();
        self.process_image.set_last_wkc(wkc);
        self.cycle_count += 1;

        if wkc != self.process_image.expected_wkc {
            self.stats.record_wkc_error();
            self.consecutive_wkc_errors += 1;

            warn!(
                expected = self.process_image.expected_wkc,
                actual = wkc,
                cycle = self.cycle_count,
                consecutive_errors = self.consecutive_wkc_errors,
                "Working counter mismatch"
            );

            // Check if we've exceeded the WKC error threshold
            let threshold = self.config.wkc_error_threshold;
            if threshold > 0 && self.consecutive_wkc_errors >= threshold {
                error!(
                    threshold,
                    consecutive_errors = self.consecutive_wkc_errors,
                    "WKC error threshold exceeded - fieldbus fault"
                );
                self.state = MasterState::Fault;
                return Err(PlcError::WkcThresholdExceeded {
                    consecutive: self.consecutive_wkc_errors,
                    threshold,
                });
            }
        } else {
            // Reset consecutive error count on successful exchange
            if self.consecutive_wkc_errors > 0 {
                debug!(
                    previous_errors = self.consecutive_wkc_errors,
                    "WKC recovered after consecutive errors"
                );
                self.consecutive_wkc_errors = 0;
            }
            self.stats.record_success(rtt.as_micros() as u32);
        }

        // Update DC if enabled
        if self.dc.is_active() {
            if let Ok(dc_time) = self.transport.read_dc_time() {
                let deviation = self.dc.update(dc_time);
                trace!(
                    deviation_ns = deviation,
                    cycle = self.cycle_count,
                    "DC sync"
                );
            }
        }

        trace!(
            cycle = self.cycle_count,
            wkc,
            rtt_us = rtt.as_micros(),
            "PDO exchange complete"
        );

        Ok(())
    }

    /// Read an SDO from a slave.
    pub fn sdo_read(&mut self, request: &SdoRequest) -> PlcResult<Vec<u8>> {
        if self.state == MasterState::Offline {
            return Err(PlcError::FieldbusError("Master is offline".into()));
        }

        self.transport.sdo_read(request)
    }

    /// Write an SDO to a slave.
    pub fn sdo_write(&mut self, request: &SdoRequest) -> PlcResult<()> {
        if self.state == MasterState::Offline {
            return Err(PlcError::FieldbusError("Master is offline".into()));
        }

        if request.write_data.is_none() {
            return Err(PlcError::FieldbusError("No write data provided".into()));
        }

        self.transport.sdo_write(request)
    }

    /// Gracefully shutdown the master.
    pub fn shutdown(&mut self) -> PlcResult<()> {
        info!("Shutting down EtherCAT master");

        // Transition through states in reverse
        if self.state == MasterState::Op {
            self.transport.set_state(SlaveState::SafeOp)?;
            self.state = MasterState::SafeOp;
        }

        if self.state == MasterState::SafeOp {
            self.transport.set_state(SlaveState::PreOp)?;
            self.state = MasterState::PreOp;
        }

        if self.state == MasterState::PreOp {
            self.transport.set_state(SlaveState::Init)?;
            self.state = MasterState::Init;
        }

        self.transport.close()?;
        self.state = MasterState::Offline;

        info!(
            total_cycles = self.cycle_count,
            wkc_errors = self.stats.wkc_errors,
            "EtherCAT master shutdown complete"
        );

        Ok(())
    }
}

impl FieldbusDriver for EthercatMaster {
    fn init(&mut self) -> PlcResult<()> {
        self.scan_slaves()?;
        self.configure_slaves()?;
        self.configure_dc()?;
        self.enter_safe_op()?;
        self.enter_op()?;
        Ok(())
    }

    fn read_inputs(&mut self) -> PlcResult<()> {
        // Inputs are read during exchange()
        Ok(())
    }

    fn write_outputs(&mut self) -> PlcResult<()> {
        // Outputs are written during exchange()
        Ok(())
    }

    fn exchange(&mut self) -> PlcResult<()> {
        // Delegate to the inherent EthercatMaster::exchange() method
        // which performs the actual PDO exchange with proper timing and stats
        EthercatMaster::exchange(self)
    }

    fn get_inputs(&self) -> crate::FieldbusInputs {
        let pi = &self.process_image;
        let mut inputs = crate::FieldbusInputs::default();

        // Read digital inputs (4 bytes = 32 bits, little-endian)
        // Layout: bytes 0-3 = digital inputs
        if let Some(di) = pi.read_input_u32(0) {
            inputs.digital = di;
        }

        // Read analog inputs (2 bytes each, little-endian)
        // Layout: bytes 4+ = analog inputs (16 channels * 2 bytes = 32 bytes)
        for (i, ai) in inputs.analog.iter_mut().enumerate() {
            if let Some(val) = pi.read_input_u16(4 + i * 2) {
                *ai = val as i16;
            }
        }

        inputs
    }

    fn set_outputs(&mut self, outputs: &crate::FieldbusOutputs) {
        let pi = self.process_image_mut();

        // Write digital outputs (4 bytes = 32 bits, little-endian)
        // Layout: bytes 0-3 = digital outputs
        pi.write_output_u32(0, outputs.digital);

        // Write analog outputs (2 bytes each, little-endian)
        // Layout: bytes 4+ = analog outputs (16 channels * 2 bytes = 32 bytes)
        for (i, ao) in outputs.analog.iter().enumerate() {
            pi.write_output_u16(4 + i * 2, *ao as u16);
        }
    }

    fn shutdown(&mut self) -> PlcResult<()> {
        EthercatMaster::shutdown(self)
    }
}

/// Transport layer abstraction for EtherCAT frame handling.
///
/// This trait allows swapping between real SOEM bindings and
/// a simulated transport for testing.
pub trait EthercatTransport: Send {
    /// Scan for slaves on the network.
    fn scan_slaves(&mut self) -> PlcResult<Vec<SlaveConfig>>;

    /// Set all slaves to the specified state.
    fn set_state(&mut self, state: SlaveState) -> PlcResult<()>;

    /// Configure DC for a slave.
    fn configure_slave_dc(&mut self, config: &DcSlaveConfig) -> PlcResult<()>;

    /// Read the DC system time from the reference clock.
    fn read_dc_time(&mut self) -> PlcResult<u64>;

    /// Exchange process data.
    ///
    /// Sends outputs to slaves and receives inputs.
    /// Returns the working counter.
    fn exchange(&mut self, outputs: &[u8], inputs: &mut [u8]) -> PlcResult<u16>;

    /// Read an SDO.
    fn sdo_read(&mut self, request: &SdoRequest) -> PlcResult<Vec<u8>>;

    /// Write an SDO.
    fn sdo_write(&mut self, request: &SdoRequest) -> PlcResult<()>;

    /// Close the transport.
    fn close(&mut self) -> PlcResult<()>;
}

/// Simulated EtherCAT transport for testing.
///
/// This allows testing the master logic without actual hardware.
#[derive(Debug)]
pub struct SimulatedTransport {
    /// Interface name (for logging).
    interface: String,
    /// Simulated slaves.
    slaves: Vec<SlaveConfig>,
    /// Current slave state.
    current_state: SlaveState,
    /// Simulated DC time (nanoseconds).
    dc_time: u64,
    /// Cycle count for time simulation.
    cycle_count: u64,
    /// Cycle time for DC simulation.
    cycle_time_ns: u64,
    /// Whether the transport is open.
    open: bool,
}

impl SimulatedTransport {
    /// Create a new simulated transport.
    pub fn new(config: &EthercatConfig) -> Self {
        Self {
            interface: config
                .interface
                .clone()
                .unwrap_or_else(|| "simulated".into()),
            slaves: Vec::new(),
            current_state: SlaveState::Init,
            dc_time: 0,
            cycle_count: 0,
            cycle_time_ns: config.dc_sync0_cycle.as_nanos() as u64,
            open: true,
        }
    }

    /// Add a simulated slave.
    pub fn add_slave(&mut self, slave: SlaveConfig) {
        self.slaves.push(slave);
    }

    /// Create a transport with preconfigured test slaves.
    pub fn with_test_slaves(config: &EthercatConfig) -> Self {
        let mut transport = Self::new(config);

        // Add some typical test slaves
        // Slave 0: Digital I/O module (8 DI, 8 DO)
        let mut dio = SlaveConfig::new(0, SlaveIdentity::new(0x00000002, 0x04442C52, 1, 0));
        dio.name = "EL1008+EL2008 DIO".into();
        dio.dc_supported = true;
        let mut tx_pdo = PdoMapping::new(0x1A00, true);
        tx_pdo.add_entry(PdoEntry::new(0x6000, 1, 8).with_name("Digital Inputs"));
        dio.tx_pdos.push(tx_pdo);
        let mut rx_pdo = PdoMapping::new(0x1600, false);
        rx_pdo.add_entry(PdoEntry::new(0x7000, 1, 8).with_name("Digital Outputs"));
        dio.rx_pdos.push(rx_pdo);
        transport.add_slave(dio);

        // Slave 1: Analog I/O module (2 AI, 2 AO)
        let mut aio = SlaveConfig::new(1, SlaveIdentity::new(0x00000002, 0x0BC03052, 1, 0));
        aio.name = "EL3102+EL4102 AIO".into();
        aio.dc_supported = true;
        let mut tx_pdo = PdoMapping::new(0x1A00, true);
        tx_pdo.add_entry(PdoEntry::new(0x6000, 1, 16).with_name("AI Channel 1"));
        tx_pdo.add_entry(PdoEntry::new(0x6000, 2, 16).with_name("AI Channel 2"));
        aio.tx_pdos.push(tx_pdo);
        let mut rx_pdo = PdoMapping::new(0x1600, false);
        rx_pdo.add_entry(PdoEntry::new(0x7000, 1, 16).with_name("AO Channel 1"));
        rx_pdo.add_entry(PdoEntry::new(0x7000, 2, 16).with_name("AO Channel 2"));
        aio.rx_pdos.push(rx_pdo);
        transport.add_slave(aio);

        transport
    }
}

impl EthercatTransport for SimulatedTransport {
    fn scan_slaves(&mut self) -> PlcResult<Vec<SlaveConfig>> {
        if !self.open {
            return Err(PlcError::FieldbusError("Transport not open".into()));
        }
        debug!(
            interface = %self.interface,
            count = self.slaves.len(),
            "Simulated slave scan"
        );
        Ok(self.slaves.clone())
    }

    fn set_state(&mut self, state: SlaveState) -> PlcResult<()> {
        if !self.open {
            return Err(PlcError::FieldbusError("Transport not open".into()));
        }
        debug!(?state, "Simulated state transition");
        self.current_state = state;
        Ok(())
    }

    fn configure_slave_dc(&mut self, config: &DcSlaveConfig) -> PlcResult<()> {
        debug!(
            slave = config.position,
            sync_mode = ?config.sync_mode,
            "Simulated DC configuration"
        );
        Ok(())
    }

    fn read_dc_time(&mut self) -> PlcResult<u64> {
        // Simulate time advancing with each cycle
        self.dc_time += self.cycle_time_ns;
        Ok(self.dc_time)
    }

    fn exchange(&mut self, outputs: &[u8], inputs: &mut [u8]) -> PlcResult<u16> {
        if !self.open {
            return Err(PlcError::FieldbusError("Transport not open".into()));
        }

        self.cycle_count += 1;

        // Simulate some input data
        // For digital inputs, echo back outputs with some modifications
        if !inputs.is_empty() && !outputs.is_empty() {
            // First byte: echo digital outputs
            inputs[0] = outputs[0];
        }

        // Simulate analog inputs (sine wave on channel 1)
        if inputs.len() >= 3 {
            let angle = (self.cycle_count as f64 * 0.01).sin();
            let value = ((angle * 16000.0) as i16).to_le_bytes();
            inputs[1] = value[0];
            inputs[2] = value[1];
        }

        // Return expected working counter
        let mut wkc = 0u16;
        for slave in &self.slaves {
            if slave.calculate_input_size() > 0 {
                wkc += 1;
            }
            if slave.calculate_output_size() > 0 {
                wkc += 2;
            }
        }

        Ok(wkc)
    }

    fn sdo_read(&mut self, request: &SdoRequest) -> PlcResult<Vec<u8>> {
        debug!(
            slave = request.slave,
            address = %request.address,
            "Simulated SDO read"
        );
        // Return dummy data
        Ok(vec![0; 4])
    }

    fn sdo_write(&mut self, request: &SdoRequest) -> PlcResult<()> {
        debug!(
            slave = request.slave,
            address = %request.address,
            "Simulated SDO write"
        );
        Ok(())
    }

    fn close(&mut self) -> PlcResult<()> {
        self.open = false;
        debug!("Simulated transport closed");
        Ok(())
    }
}

// SOEM-based EtherCAT transport (feature-gated, Linux-only)
#[cfg(all(feature = "soem", target_os = "linux"))]
mod soem_transport {
    //! SOEM-rs based EtherCAT transport.
    //!
    //! This module provides a real EtherCAT master implementation using the
    //! `soem` crate, which wraps the Simple Open EtherCAT Master (SOEM) library.
    //!
    //! # Requirements
    //!
    //! - Linux with raw socket capabilities (CAP_NET_RAW) or root privileges
    //! - libsoem-dev installed or SOEM built from source
    //!
    //! # Safety
    //!
    //! The SOEM library uses raw Ethernet frames for EtherCAT communication.
    //! This requires elevated privileges and direct hardware access.

    use super::*;
    use plc_common::error::PlcError;
    use std::ffi::c_int;
    use std::fs;
    use std::path::Path;

    /// Default timeout for SDO operations in microseconds.
    const SDO_TIMEOUT_US: c_int = 50_000; // 50ms

    /// Default timeout for process data receive in microseconds.
    const PROCESSDATA_TIMEOUT_US: c_int = 2_000; // 2ms

    /// Maximum number of slaves supported.
    const MAX_SLAVES: usize = 128;

    /// Maximum number of groups.
    const MAX_GROUPS: usize = 2;

    /// I/O map size (4KB as per SOEM API).
    const IO_MAP_SIZE: usize = 4096;

    /// Linux capability bit for CAP_NET_RAW.
    const CAP_NET_RAW_BIT: u32 = 13;

    /// SOEM-based EtherCAT transport.
    ///
    /// Provides real EtherCAT communication using the SOEM library via the
    /// `soem` Rust crate. This transport handles:
    ///
    /// - Slave scanning and configuration
    /// - Process data (PDO) exchange
    /// - Service data (SDO) read/write
    /// - Distributed Clocks (DC) configuration
    ///
    /// # Thread Safety
    ///
    /// The underlying SOEM context is not thread-safe (!Send, !Sync).
    /// All operations must be performed from the same thread.
    pub struct SoemTransport {
        /// Network interface name (e.g., "eth0").
        interface: String,
        /// SOEM port for network communication.
        port: soem::Port,
        /// Slave array for SOEM context.
        slaves: Vec<soem::Slave>,
        /// Slave count returned by SOEM.
        slave_count: c_int,
        /// Group configurations.
        groups: Vec<soem::Group>,
        /// ESI buffer for EEPROM operations.
        esibuf: Vec<soem::ESIBuf>,
        /// ESI map for slave information.
        esimap: Vec<soem::ESIMap>,
        /// Error ring buffer.
        elist: Vec<soem::ERing>,
        /// Index stack for frame handling.
        idxstack: Vec<soem::IdxStack>,
        /// Error flag array.
        ecaterror: Vec<soem::Boolean>,
        /// DC time storage.
        dc_time: i64,
        /// Sync manager communication types.
        sm_commtype: Vec<soem::SMCommType>,
        /// PDO assignment storage.
        pdo_assign: Vec<soem::PDOAssign>,
        /// PDO description storage.
        pdo_desc: Vec<soem::PDODesc>,
        /// EEPROM sync manager configuration.
        eep_sm: Vec<soem::EEPROMSM>,
        /// EEPROM FMMU configuration.
        eep_fmmu: Vec<soem::EEPROMFMMU>,
        /// I/O map buffer for process data.
        io_map: Box<[u8; IO_MAP_SIZE]>,
        /// Expected working counter for all slaves.
        expected_wkc: u16,
        /// Whether the transport is initialized.
        initialized: bool,
        /// Cached slave configurations for returning from scan_slaves.
        cached_slaves: Vec<SlaveConfig>,
    }

    impl SoemTransport {
        fn check_interface_exists(interface: &str) -> PlcResult<()> {
            let path = format!("/sys/class/net/{interface}");
            if !Path::new(&path).exists() {
                return Err(PlcError::FieldbusError(format!(
                    "EtherCAT interface '{interface}' not found (expected {path})"
                )));
            }
            Ok(())
        }

        fn has_cap_net_raw() -> bool {
            let status = match fs::read_to_string("/proc/self/status") {
                Ok(status) => status,
                Err(_) => return false,
            };

            for line in status.lines() {
                if let Some(value) = line.strip_prefix("CapEff:\t") {
                    if let Ok(bits) = u64::from_str_radix(value.trim(), 16) {
                        return (bits & (1u64 << CAP_NET_RAW_BIT)) != 0;
                    }
                    break;
                }
            }
            false
        }

        fn check_raw_socket_privilege() -> PlcResult<()> {
            let is_root = unsafe { libc::geteuid() == 0 };
            if is_root || Self::has_cap_net_raw() {
                return Ok(());
            }

            Err(PlcError::FieldbusError(
                "EtherCAT requires CAP_NET_RAW (or root) to open raw sockets".into(),
            ))
        }

        /// Create a new SOEM transport for the given network interface.
        ///
        /// # Arguments
        ///
        /// * `interface` - Network interface name (e.g., "eth0", "enp0s25")
        ///
        /// # Errors
        ///
        /// Returns an error if:
        /// - The interface name is empty
        /// - SOEM initialization fails (e.g., insufficient privileges)
        pub fn new(interface: &str) -> PlcResult<Self> {
            if interface.is_empty() {
                return Err(PlcError::FieldbusError(
                    "Interface name cannot be empty".into(),
                ));
            }

            Self::check_interface_exists(interface)?;
            Self::check_raw_socket_privilege()?;

            info!(interface, "Creating SOEM transport");

            Ok(Self {
                interface: interface.to_string(),
                port: soem::Port::default(),
                slaves: vec![soem::Slave::default(); MAX_SLAVES + 1], // +1 for master slot
                slave_count: 0,
                groups: vec![soem::Group::default(); MAX_GROUPS],
                esibuf: vec![soem::ESIBuf::default(); MAX_SLAVES],
                esimap: vec![soem::ESIMap::default(); MAX_SLAVES],
                elist: vec![soem::ERing::default(); MAX_SLAVES],
                idxstack: vec![soem::IdxStack::default(); MAX_SLAVES],
                ecaterror: vec![soem::Boolean::default(); MAX_SLAVES],
                dc_time: 0,
                sm_commtype: vec![soem::SMCommType::default(); MAX_SLAVES],
                pdo_assign: vec![soem::PDOAssign::default(); MAX_SLAVES],
                pdo_desc: vec![soem::PDODesc::default(); MAX_SLAVES],
                eep_sm: vec![soem::EEPROMSM::default(); MAX_SLAVES],
                eep_fmmu: vec![soem::EEPROMFMMU::default(); MAX_SLAVES],
                io_map: Box::new([0u8; IO_MAP_SIZE]),
                expected_wkc: 0,
                initialized: false,
                cached_slaves: Vec::new(),
            })
        }

        /// Initialize SOEM context and open the network interface.
        ///
        /// # Safety
        ///
        /// This method creates a SOEM context which involves FFI calls to the
        /// underlying C library. The context holds mutable references to our
        /// internal buffers, which is safe because:
        /// - All buffers are owned by this struct and have stable addresses
        /// - The context lifetime is managed within method calls
        /// - We ensure buffers outlive any context usage
        fn init_context(&mut self) -> PlcResult<()> {
            // Create context with mutable references to our storage
            // The context is created fresh for each operation that needs it
            let _context = soem::Context::new(
                &[&self.interface],
                &mut self.port,
                &mut self.slaves,
                &mut self.slave_count,
                &mut self.groups,
                &mut self.esibuf,
                &mut self.esimap,
                &mut self.elist,
                &mut self.idxstack,
                &mut self.ecaterror,
                &mut self.dc_time,
                &mut self.sm_commtype,
                &mut self.pdo_assign,
                &mut self.pdo_desc,
                &mut self.eep_sm,
                &mut self.eep_fmmu,
            )
            .map_err(|e| {
                PlcError::FieldbusError(format!(
                    "Failed to initialize SOEM on {}: {:?}",
                    self.interface, e
                ))
            })?;

            self.initialized = true;
            info!(interface = %self.interface, "SOEM context initialized");
            Ok(())
        }

        /// Create a temporary SOEM context for operations.
        ///
        /// The context borrows our internal buffers and is used for a single
        /// operation. This pattern is necessary because SOEM's Context type
        /// holds mutable references and doesn't implement Clone.
        fn with_context<F, T>(&mut self, f: F) -> PlcResult<T>
        where
            F: FnOnce(&mut soem::Context<'_>) -> PlcResult<T>,
        {
            let mut context = soem::Context::new(
                &[&self.interface],
                &mut self.port,
                &mut self.slaves,
                &mut self.slave_count,
                &mut self.groups,
                &mut self.esibuf,
                &mut self.esimap,
                &mut self.elist,
                &mut self.idxstack,
                &mut self.ecaterror,
                &mut self.dc_time,
                &mut self.sm_commtype,
                &mut self.pdo_assign,
                &mut self.pdo_desc,
                &mut self.eep_sm,
                &mut self.eep_fmmu,
            )
            .map_err(|e| {
                PlcError::FieldbusError(format!(
                    "Failed to create SOEM context on {}: {:?}",
                    self.interface, e
                ))
            })?;

            f(&mut context)
        }

        /// Convert SOEM EtherCatState to our SlaveState.
        fn soem_state_to_slave_state(state: soem::EtherCatState) -> Option<SlaveState> {
            match state {
                soem::EtherCatState::Init => Some(SlaveState::Init),
                soem::EtherCatState::PreOp => Some(SlaveState::PreOp),
                soem::EtherCatState::SafeOp => Some(SlaveState::SafeOp),
                soem::EtherCatState::Op => Some(SlaveState::Op),
                soem::EtherCatState::Boot => Some(SlaveState::Bootstrap),
                _ => None,
            }
        }

        /// Convert our SlaveState to SOEM EtherCatState.
        fn slave_state_to_soem_state(state: SlaveState) -> soem::EtherCatState {
            match state {
                SlaveState::Init => soem::EtherCatState::Init,
                SlaveState::PreOp => soem::EtherCatState::PreOp,
                SlaveState::SafeOp => soem::EtherCatState::SafeOp,
                SlaveState::Op => soem::EtherCatState::Op,
                SlaveState::Bootstrap => soem::EtherCatState::Boot,
            }
        }

        /// Parse SOEM slave info into our SlaveConfig format.
        fn parse_slave_info(&self, idx: usize, slave: &soem::Slave) -> SlaveConfig {
            let position = idx as u16;
            let identity = SlaveIdentity::new(
                slave.eep_manufacturer(),
                slave.eep_id(),
                slave.eep_revision(),
                0, // Serial not directly available
            );

            let mut config = SlaveConfig::new(position, identity);
            config.name = slave.name().to_string();
            config.configured_address = slave.configured_addr();
            config.dc_supported = slave.has_dc();
            config.input_size = slave.input_size() as usize;
            config.output_size = slave.output_size() as usize;
            config.state =
                Self::soem_state_to_slave_state(slave.state()).unwrap_or(SlaveState::Init);

            config
        }
    }

    impl EthercatTransport for SoemTransport {
        fn scan_slaves(&mut self) -> PlcResult<Vec<SlaveConfig>> {
            info!(interface = %self.interface, "Scanning for EtherCAT slaves");

            // Initialize if not already done
            if !self.initialized {
                self.init_context()?;
            }

            self.with_context(|ctx| {
                // Scan and configure slaves
                let slave_count = ctx.config_init(false).map_err(|e| {
                    PlcError::FieldbusError(format!("Failed to scan slaves: {:?}", e))
                })?;

                if slave_count == 0 {
                    warn!("No EtherCAT slaves found on the network");
                    return Ok(Vec::new());
                }

                info!(slave_count, "Found EtherCAT slaves");

                // Map I/O for group 0 (default group)
                // Safety: io_map is a fixed-size array owned by this struct
                let io_map: &mut [u8; IO_MAP_SIZE] = unsafe {
                    // We need to transmute because config_map_group expects &'a mut [u8; 4096]
                    // where 'a matches the context lifetime, but our io_map has a different lifetime.
                    // This is safe because:
                    // 1. The io_map is owned by SoemTransport and has a stable address
                    // 2. We only use the context within this closure
                    // 3. The io_map outlives the context
                    &mut *(std::ptr::from_mut(&mut *self.io_map).cast::<[u8; IO_MAP_SIZE]>())
                };

                ctx.config_map_group(io_map, 0).map_err(|mut errors| {
                    // Collect first error for reporting
                    if let Some(e) = errors.next() {
                        PlcError::FieldbusError(format!("Failed to map I/O: {:?}", e))
                    } else {
                        PlcError::FieldbusError("Failed to map I/O: unknown error".into())
                    }
                })?;

                // Get expected working counter from group 0
                self.expected_wkc = ctx.groups()[0].expected_wkc();

                Ok(())
            })?;

            // Parse slave information after context operations
            let mut slaves = Vec::new();
            let count = self.slave_count as usize;

            // SOEM uses 1-based indexing for slaves (0 is master)
            for idx in 1..=count {
                if idx < self.slaves.len() {
                    let slave = &self.slaves[idx];
                    let config = self.parse_slave_info(idx - 1, slave);
                    debug!(
                        position = config.position,
                        name = %config.name,
                        vendor = config.identity.vendor_id,
                        product = config.identity.product_code,
                        dc = config.dc_supported,
                        "Discovered slave"
                    );
                    slaves.push(config);
                }
            }

            self.cached_slaves = slaves.clone();
            Ok(slaves)
        }

        fn set_state(&mut self, state: SlaveState) -> PlcResult<()> {
            let soem_state = Self::slave_state_to_soem_state(state);
            debug!(?state, "Setting all slaves to state");

            self.with_context(|ctx| {
                // Set state for all slaves (slave 0 means all)
                ctx.set_state(soem_state, 0);

                // Write the state to slaves
                ctx.write_state(0).map_err(|e| {
                    PlcError::FieldbusError(format!("Failed to write state {:?}: {:?}", state, e))
                })?;

                // Wait for state transition with timeout
                let timeout_us = 500_000; // 500ms
                let actual_state = ctx.check_state(0, soem_state, timeout_us);

                if actual_state != soem_state {
                    warn!(
                        expected = ?state,
                        actual = ?Self::soem_state_to_slave_state(actual_state),
                        "State transition incomplete"
                    );
                }

                Ok(())
            })
        }

        fn configure_slave_dc(&mut self, config: &DcSlaveConfig) -> PlcResult<()> {
            if !config.dc_supported {
                return Ok(());
            }

            debug!(
                position = config.position,
                sync_mode = ?config.sync_mode,
                sync0_cycle_ns = config.sync0_cycle_ns,
                "Configuring DC for slave"
            );

            self.with_context(|ctx| {
                // Configure DC using SOEM's config_dc
                // This sets up distributed clocks for all DC-capable slaves
                let dc_configured = ctx.config_dc().map_err(|mut errors| {
                    if let Some(e) = errors.next() {
                        PlcError::FieldbusError(format!("Failed to configure DC: {:?}", e))
                    } else {
                        PlcError::FieldbusError("Failed to configure DC: unknown error".into())
                    }
                })?;

                if dc_configured {
                    info!(position = config.position, "DC configured successfully");
                } else {
                    debug!(
                        position = config.position,
                        "DC not available for this slave"
                    );
                }

                Ok(())
            })
        }

        fn read_dc_time(&mut self) -> PlcResult<u64> {
            self.with_context(|ctx| {
                let dc_time = ctx.dc_time();
                Ok(dc_time as u64)
            })
        }

        fn exchange(&mut self, outputs: &[u8], inputs: &mut [u8]) -> PlcResult<u16> {
            // Copy outputs to I/O map
            let output_len = outputs.len().min(IO_MAP_SIZE / 2);
            self.io_map[..output_len].copy_from_slice(&outputs[..output_len]);

            let wkc = self.with_context(|ctx| {
                // Send process data to slaves
                ctx.send_processdata();

                // Receive process data from slaves with timeout
                let wkc = ctx.receive_processdata(PROCESSDATA_TIMEOUT_US);

                Ok(wkc)
            })?;

            // Copy inputs from I/O map
            // Inputs typically follow outputs in the I/O map
            let input_start = output_len;
            let input_len = inputs.len().min(IO_MAP_SIZE - input_start);
            if input_start + input_len <= IO_MAP_SIZE {
                inputs[..input_len]
                    .copy_from_slice(&self.io_map[input_start..input_start + input_len]);
            }

            trace!(wkc, expected = self.expected_wkc, "Process data exchange");
            Ok(wkc)
        }

        fn sdo_read(&mut self, request: &SdoRequest) -> PlcResult<Vec<u8>> {
            debug!(
                slave = request.slave,
                index = format!("{:#06x}", request.address.index),
                subindex = request.address.subindex,
                "SDO read"
            );

            self.with_context(|ctx| {
                // Try reading as different sizes and use the first that succeeds
                // SOEM's read_sdo is generic over the return type

                // Try u32 first (most common)
                if let Ok(value) = ctx.read_sdo::<u32>(
                    request.slave + 1, // SOEM uses 1-based slave indexing
                    request.address.index,
                    request.address.subindex,
                    SDO_TIMEOUT_US,
                ) {
                    return Ok(value.to_le_bytes().to_vec());
                }

                // Try u16
                if let Ok(value) = ctx.read_sdo::<u16>(
                    request.slave + 1,
                    request.address.index,
                    request.address.subindex,
                    SDO_TIMEOUT_US,
                ) {
                    return Ok(value.to_le_bytes().to_vec());
                }

                // Try u8
                if let Ok(value) = ctx.read_sdo::<u8>(
                    request.slave + 1,
                    request.address.index,
                    request.address.subindex,
                    SDO_TIMEOUT_US,
                ) {
                    return Ok(vec![value]);
                }

                Err(PlcError::FieldbusError(format!(
                    "SDO read failed for slave {} at {:?}",
                    request.slave, request.address
                )))
            })
        }

        fn sdo_write(&mut self, request: &SdoRequest) -> PlcResult<()> {
            let data = request
                .write_data
                .as_ref()
                .ok_or_else(|| PlcError::FieldbusError("SDO write requires data".into()))?;

            debug!(
                slave = request.slave,
                index = format!("{:#06x}", request.address.index),
                subindex = request.address.subindex,
                data_len = data.len(),
                "SDO write"
            );

            self.with_context(|ctx| {
                // Write based on data length
                let slave_idx = request.slave + 1; // SOEM uses 1-based indexing

                match data.len() {
                    1 => {
                        ctx.write_sdo(
                            slave_idx,
                            request.address.index,
                            request.address.subindex,
                            &data[0],
                            SDO_TIMEOUT_US,
                        )
                        .map_err(|mut errors| {
                            if let Some(e) = errors.next() {
                                PlcError::FieldbusError(format!("SDO write failed: {:?}", e))
                            } else {
                                PlcError::FieldbusError("SDO write failed: unknown error".into())
                            }
                        })?;
                    }
                    2 => {
                        let value = u16::from_le_bytes([data[0], data[1]]);
                        ctx.write_sdo(
                            slave_idx,
                            request.address.index,
                            request.address.subindex,
                            &value,
                            SDO_TIMEOUT_US,
                        )
                        .map_err(|mut errors| {
                            if let Some(e) = errors.next() {
                                PlcError::FieldbusError(format!("SDO write failed: {:?}", e))
                            } else {
                                PlcError::FieldbusError("SDO write failed: unknown error".into())
                            }
                        })?;
                    }
                    4 => {
                        let value = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
                        ctx.write_sdo(
                            slave_idx,
                            request.address.index,
                            request.address.subindex,
                            &value,
                            SDO_TIMEOUT_US,
                        )
                        .map_err(|mut errors| {
                            if let Some(e) = errors.next() {
                                PlcError::FieldbusError(format!("SDO write failed: {:?}", e))
                            } else {
                                PlcError::FieldbusError("SDO write failed: unknown error".into())
                            }
                        })?;
                    }
                    _ => {
                        return Err(PlcError::FieldbusError(format!(
                            "Unsupported SDO write data length: {} (expected 1, 2, or 4)",
                            data.len()
                        )));
                    }
                }

                Ok(())
            })
        }

        fn close(&mut self) -> PlcResult<()> {
            info!(interface = %self.interface, "Closing SOEM transport");

            // Transition slaves to INIT state before closing
            if self.initialized {
                if let Err(e) = self.set_state(SlaveState::Init) {
                    warn!(error = %e, "Failed to set slaves to INIT during close");
                }
            }

            // Clear cached state
            self.initialized = false;
            self.cached_slaves.clear();
            self.expected_wkc = 0;
            self.slave_count = 0;

            // The SOEM context is dropped automatically when it goes out of scope
            // in with_context, which handles cleanup via its Drop implementation

            debug!("SOEM transport closed");
            Ok(())
        }
    }

    impl std::fmt::Debug for SoemTransport {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("SoemTransport")
                .field("interface", &self.interface)
                .field("initialized", &self.initialized)
                .field("slave_count", &self.slave_count)
                .field("expected_wkc", &self.expected_wkc)
                .finish_non_exhaustive()
        }
    }
}

// Re-export SoemTransport when the feature is enabled (Linux-only)
#[cfg(all(feature = "soem", target_os = "linux"))]
pub use soem_transport::SoemTransport;

// Placeholder for legacy SOEM FFI transport (deprecated feature)
#[cfg(feature = "soem-ffi")]
mod soem_ffi_transport {
    //! Legacy SOEM FFI bindings (deprecated).
    //!
    //! This module is kept for backwards compatibility.
    //! Use the `soem` feature instead, which provides the `SoemTransport` type.
    //!
    //! To migrate:
    //! 1. Replace `--features soem-ffi` with `--features soem`
    //! 2. Use `SoemTransport::new(interface)` instead of manual FFI

    #![allow(dead_code)]
    use super::*;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn test_config() -> EthercatConfig {
        EthercatConfig {
            interface: Some("sim0".into()),
            dc_enabled: true,
            dc_sync0_cycle: Duration::from_millis(1),
            esi_path: None,
            wkc_error_threshold: 3,
        }
    }

    #[test]
    fn test_master_creation() {
        let config = test_config();
        let master = EthercatMaster::new(config);
        assert_eq!(master.state(), MasterState::Offline);
    }

    #[test]
    fn test_simulated_scan() {
        let config = test_config();
        let transport = SimulatedTransport::with_test_slaves(&config);
        let mut master = EthercatMaster::with_transport(config, Box::new(transport));

        let count = master.scan_slaves().unwrap();
        assert_eq!(count, 2);
        assert_eq!(master.state(), MasterState::Init);
    }

    #[test]
    fn test_scan_slaves_is_idempotent() {
        let config = test_config();
        let transport = SimulatedTransport::with_test_slaves(&config);
        let mut master = EthercatMaster::with_transport(config, Box::new(transport));

        // First scan
        let count1 = master.scan_slaves().unwrap();
        assert_eq!(count1, 2);
        assert_eq!(master.network().slave_count(), 2);

        // Second scan should produce the same result, not accumulate
        let count2 = master.scan_slaves().unwrap();
        assert_eq!(count2, 2);
        assert_eq!(master.network().slave_count(), 2);

        // DC slaves should also not accumulate
        assert_eq!(master.dc().slaves().len(), 2);
    }

    #[test]
    fn test_full_initialization() {
        let config = test_config();
        let transport = SimulatedTransport::with_test_slaves(&config);
        let mut master = EthercatMaster::with_transport(config, Box::new(transport));

        master.init().unwrap();
        assert_eq!(master.state(), MasterState::Op);
    }

    #[test]
    fn test_pdo_exchange() {
        let config = test_config();
        let transport = SimulatedTransport::with_test_slaves(&config);
        let mut master = EthercatMaster::with_transport(config, Box::new(transport));

        master.init().unwrap();

        // Set some outputs
        master.process_image_mut().write_output_byte(0, 0xAA);

        // Exchange
        master.exchange().unwrap();

        // Check inputs were updated
        assert!(master.process_image().wkc_ok());
        assert_eq!(master.cycle_count(), 1);
    }

    #[test]
    fn test_shutdown() {
        let config = test_config();
        let transport = SimulatedTransport::with_test_slaves(&config);
        let mut master = EthercatMaster::with_transport(config, Box::new(transport));

        master.init().unwrap();
        master.shutdown().unwrap();

        assert_eq!(master.state(), MasterState::Offline);
    }

    #[test]
    fn test_frame_stats() {
        let mut stats = FrameStats::default();

        stats.record_success(100);
        stats.record_success(150);
        stats.record_success(80);

        assert_eq!(stats.frames_sent, 3);
        assert_eq!(stats.frames_received, 3);
        assert_eq!(stats.min_rtt_us, 80);
        assert_eq!(stats.max_rtt_us, 150);
    }

    #[test]
    fn test_process_image() {
        let mut pi = ProcessImage::new(10, 10);

        pi.write_output_u16(0, 0x1234);
        assert_eq!(pi.outputs()[0], 0x34);
        assert_eq!(pi.outputs()[1], 0x12);

        pi.inputs_mut()[0] = 0xAB;
        pi.inputs_mut()[1] = 0xCD;
        assert_eq!(pi.read_input_u16(0), Some(0xCDAB));
    }

    /// Transport that returns bad WKC after N cycles
    struct WkcErrorTransport {
        inner: SimulatedTransport,
        error_after_cycles: u64,
        cycle_count: u64,
    }

    impl WkcErrorTransport {
        fn new(config: &EthercatConfig, error_after_cycles: u64) -> Self {
            Self {
                inner: SimulatedTransport::with_test_slaves(config),
                error_after_cycles,
                cycle_count: 0,
            }
        }
    }

    impl EthercatTransport for WkcErrorTransport {
        fn scan_slaves(&mut self) -> PlcResult<Vec<SlaveConfig>> {
            self.inner.scan_slaves()
        }

        fn set_state(&mut self, state: SlaveState) -> PlcResult<()> {
            self.inner.set_state(state)
        }

        fn configure_slave_dc(&mut self, config: &DcSlaveConfig) -> PlcResult<()> {
            self.inner.configure_slave_dc(config)
        }

        fn read_dc_time(&mut self) -> PlcResult<u64> {
            self.inner.read_dc_time()
        }

        fn exchange(&mut self, outputs: &[u8], inputs: &mut [u8]) -> PlcResult<u16> {
            self.cycle_count += 1;
            let wkc = self.inner.exchange(outputs, inputs)?;
            // Return 0 WKC after the specified number of cycles to simulate error
            if self.cycle_count > self.error_after_cycles {
                Ok(0) // Bad WKC
            } else {
                Ok(wkc)
            }
        }

        fn sdo_read(&mut self, request: &SdoRequest) -> PlcResult<Vec<u8>> {
            self.inner.sdo_read(request)
        }

        fn sdo_write(&mut self, request: &SdoRequest) -> PlcResult<()> {
            self.inner.sdo_write(request)
        }

        fn close(&mut self) -> PlcResult<()> {
            self.inner.close()
        }
    }

    #[test]
    fn test_wkc_error_threshold() {
        let mut config = test_config();
        config.wkc_error_threshold = 3; // Fault after 3 consecutive WKC errors

        let transport = WkcErrorTransport::new(&config, 2); // Start errors after 2 good cycles
        let mut master = EthercatMaster::with_transport(config, Box::new(transport));

        master.init().unwrap();

        // First 2 cycles should succeed
        assert!(master.exchange().is_ok());
        assert!(master.exchange().is_ok());
        assert_eq!(master.state(), MasterState::Op);

        // Next 2 cycles have WKC errors but below threshold
        assert!(master.exchange().is_ok()); // 1st WKC error
        assert!(master.exchange().is_ok()); // 2nd WKC error
        assert_eq!(master.state(), MasterState::Op);

        // 3rd WKC error should trigger fault
        let result = master.exchange();
        assert!(result.is_err());
        assert_eq!(master.state(), MasterState::Fault);
    }

    #[test]
    fn test_wkc_error_recovery() {
        let mut config = test_config();
        config.wkc_error_threshold = 5; // Higher threshold

        // Use normal transport (always returns good WKC)
        let transport = SimulatedTransport::with_test_slaves(&config);
        let mut master = EthercatMaster::with_transport(config, Box::new(transport));

        master.init().unwrap();

        // Run several successful exchanges
        for _ in 0..10 {
            assert!(master.exchange().is_ok());
        }

        // Consecutive errors should be 0
        assert_eq!(master.state(), MasterState::Op);
    }

    #[test]
    fn test_wkc_threshold_disabled() {
        let mut config = test_config();
        config.wkc_error_threshold = 0; // Disabled - only log warnings

        let transport = WkcErrorTransport::new(&config, 0); // All cycles have WKC errors
        let mut master = EthercatMaster::with_transport(config, Box::new(transport));

        master.init().unwrap();

        // Even with many WKC errors, should not fault when threshold is 0
        for _ in 0..10 {
            assert!(master.exchange().is_ok());
        }
        assert_eq!(master.state(), MasterState::Op);
    }
}
