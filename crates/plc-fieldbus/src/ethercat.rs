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

        // Read digital inputs from first byte
        if let Some(di) = pi.read_input_byte(0) {
            inputs.digital = u32::from(di);
        }

        // Read analog inputs (2 bytes each, little-endian)
        for (i, ai) in inputs.analog.iter_mut().enumerate() {
            if let Some(val) = pi.read_input_u16(1 + i * 2) {
                *ai = val as i16;
            }
        }

        inputs
    }

    fn set_outputs(&mut self, outputs: &crate::FieldbusOutputs) {
        let pi = self.process_image_mut();

        // Write digital outputs to first byte
        pi.write_output_byte(0, outputs.digital as u8);

        // Write analog outputs (2 bytes each, little-endian)
        for (i, ao) in outputs.analog.iter().enumerate() {
            pi.write_output_u16(1 + i * 2, *ao as u16);
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

// Placeholder for SOEM FFI transport (feature-gated)
#[cfg(feature = "soem-ffi")]
mod soem_transport {
    //! SOEM FFI bindings.
    //!
    //! This module would contain the actual FFI bindings to the SOEM library.
    //! Requires libsoem-dev to be installed.
    //!
    //! To implement:
    //! 1. Link against libsoem via build.rs
    //! 2. Declare extern "C" functions for ec_init, ec_config, etc.
    //! 3. Implement EthercatTransport trait

    use super::*;

    // extern "C" {
    //     fn ec_init(ifname: *const c_char) -> c_int;
    //     fn ec_config(...) -> c_int;
    //     fn ec_send_processdata() -> c_int;
    //     fn ec_receive_processdata(timeout: c_int) -> c_int;
    //     fn ec_close();
    // }
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
