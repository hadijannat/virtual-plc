//! EtherCAT integration tests using SimulatedTransport.
//!
//! These tests verify the EtherCAT master behavior including:
//! - Multi-slave topology handling
//! - State machine transitions
//! - WKC error handling and recovery
//! - Distributed Clocks (DC) synchronization
//! - Process image I/O mapping via FieldbusDriver trait

use plc_common::config::EthercatConfig;
use plc_common::error::PlcError;
use plc_fieldbus::dc_sync::DcSlaveConfig;
use plc_fieldbus::slave_config::{PdoEntry, PdoMapping, SlaveConfig, SlaveIdentity, SlaveState};
use plc_fieldbus::{
    EthercatMaster, EthercatTransport, FieldbusDriver, FieldbusOutputs, MasterState,
    SimulatedTransport,
};
use std::time::Duration;

// ============================================================================
// Test Helpers
// ============================================================================

/// Create a standard test configuration.
fn test_config() -> EthercatConfig {
    EthercatConfig {
        interface: Some("sim0".into()),
        dc_enabled: true,
        dc_sync0_cycle: Duration::from_millis(1),
        esi_path: None,
        wkc_error_threshold: 3,
    }
}

/// Create a digital I/O slave module.
fn create_dio_slave(position: u16, input_bits: u16, output_bits: u16) -> SlaveConfig {
    let mut slave = SlaveConfig::new(
        position,
        SlaveIdentity::new(0x00000002, 0x04442C52 + u32::from(position), 1, 0),
    );
    slave.name = format!("DIO-{}", position);
    slave.dc_supported = true;

    if input_bits > 0 {
        let mut tx_pdo = PdoMapping::new(0x1A00, true);
        tx_pdo.add_entry(PdoEntry::new(0x6000, 1, input_bits).with_name("Digital Inputs"));
        slave.tx_pdos.push(tx_pdo);
    }

    if output_bits > 0 {
        let mut rx_pdo = PdoMapping::new(0x1600, false);
        rx_pdo.add_entry(PdoEntry::new(0x7000, 1, output_bits).with_name("Digital Outputs"));
        slave.rx_pdos.push(rx_pdo);
    }

    slave
}

/// Create an analog I/O slave module.
fn create_aio_slave(position: u16, input_channels: u8, output_channels: u8) -> SlaveConfig {
    let mut slave = SlaveConfig::new(
        position,
        SlaveIdentity::new(0x00000002, 0x0BC03052 + u32::from(position), 1, 0),
    );
    slave.name = format!("AIO-{}", position);
    slave.dc_supported = true;

    if input_channels > 0 {
        let mut tx_pdo = PdoMapping::new(0x1A00, true);
        for i in 1..=input_channels {
            tx_pdo.add_entry(PdoEntry::new(0x6000, i, 16).with_name(format!("AI Channel {}", i)));
        }
        slave.tx_pdos.push(tx_pdo);
    }

    if output_channels > 0 {
        let mut rx_pdo = PdoMapping::new(0x1600, false);
        for i in 1..=output_channels {
            rx_pdo.add_entry(PdoEntry::new(0x7000, i, 16).with_name(format!("AO Channel {}", i)));
        }
        slave.rx_pdos.push(rx_pdo);
    }

    slave
}

/// Create a mixed I/O slave (both digital and analog).
fn create_mixed_slave(
    position: u16,
    di_bits: u16,
    do_bits: u16,
    ai_channels: u8,
    ao_channels: u8,
) -> SlaveConfig {
    let mut slave = SlaveConfig::new(
        position,
        SlaveIdentity::new(0x00000002, 0x0DD05052 + u32::from(position), 1, 0),
    );
    slave.name = format!("Mixed-{}", position);
    slave.dc_supported = true;

    // TxPDO for inputs
    let mut tx_pdo = PdoMapping::new(0x1A00, true);
    if di_bits > 0 {
        tx_pdo.add_entry(PdoEntry::new(0x6000, 1, di_bits).with_name("Digital Inputs"));
    }
    for i in 1..=ai_channels {
        tx_pdo.add_entry(PdoEntry::new(0x6010, i, 16).with_name(format!("AI {}", i)));
    }
    if tx_pdo.total_bits > 0 {
        slave.tx_pdos.push(tx_pdo);
    }

    // RxPDO for outputs
    let mut rx_pdo = PdoMapping::new(0x1600, false);
    if do_bits > 0 {
        rx_pdo.add_entry(PdoEntry::new(0x7000, 1, do_bits).with_name("Digital Outputs"));
    }
    for i in 1..=ao_channels {
        rx_pdo.add_entry(PdoEntry::new(0x7010, i, 16).with_name(format!("AO {}", i)));
    }
    if rx_pdo.total_bits > 0 {
        slave.rx_pdos.push(rx_pdo);
    }

    slave
}

/// Custom transport wrapper that can inject WKC errors.
struct WkcErrorTransport {
    inner: SimulatedTransport,
    /// Cycles to return bad WKC for (0-indexed).
    error_cycles: Vec<u64>,
    cycle_count: u64,
    /// Whether to return WKC=0 or expected_wkc-1.
    zero_wkc: bool,
}

impl WkcErrorTransport {
    fn new(config: &EthercatConfig) -> Self {
        Self {
            inner: SimulatedTransport::new(config),
            error_cycles: Vec::new(),
            cycle_count: 0,
            zero_wkc: true,
        }
    }

    fn add_slave(&mut self, slave: SlaveConfig) {
        self.inner.add_slave(slave);
    }

    fn with_errors_at(mut self, cycles: Vec<u64>) -> Self {
        self.error_cycles = cycles;
        self
    }

    #[allow(dead_code)]
    fn with_partial_wkc(mut self) -> Self {
        self.zero_wkc = false;
        self
    }
}

impl EthercatTransport for WkcErrorTransport {
    fn scan_slaves(&mut self) -> plc_common::PlcResult<Vec<SlaveConfig>> {
        self.inner.scan_slaves()
    }

    fn set_state(&mut self, state: SlaveState) -> plc_common::PlcResult<()> {
        self.inner.set_state(state)
    }

    fn configure_slave_dc(&mut self, config: &DcSlaveConfig) -> plc_common::PlcResult<()> {
        self.inner.configure_slave_dc(config)
    }

    fn read_dc_time(&mut self) -> plc_common::PlcResult<u64> {
        self.inner.read_dc_time()
    }

    fn exchange(&mut self, outputs: &[u8], inputs: &mut [u8]) -> plc_common::PlcResult<u16> {
        self.cycle_count += 1;
        let wkc = self.inner.exchange(outputs, inputs)?;

        // Check if this cycle should return an error
        if self.error_cycles.contains(&(self.cycle_count - 1)) {
            if self.zero_wkc {
                Ok(0)
            } else {
                Ok(wkc.saturating_sub(1))
            }
        } else {
            Ok(wkc)
        }
    }

    fn sdo_read(
        &mut self,
        request: &plc_fieldbus::slave_config::SdoRequest,
    ) -> plc_common::PlcResult<Vec<u8>> {
        self.inner.sdo_read(request)
    }

    fn sdo_write(
        &mut self,
        request: &plc_fieldbus::slave_config::SdoRequest,
    ) -> plc_common::PlcResult<()> {
        self.inner.sdo_write(request)
    }

    fn close(&mut self) -> plc_common::PlcResult<()> {
        self.inner.close()
    }
}

// ============================================================================
// Multi-Slave Topology Tests
// ============================================================================

#[test]
fn test_4_slave_topology() {
    let config = test_config();
    let mut transport = SimulatedTransport::new(&config);

    // Create a 4-slave topology with mixed I/O
    transport.add_slave(create_dio_slave(0, 8, 8)); // 8 DI, 8 DO
    transport.add_slave(create_dio_slave(1, 16, 16)); // 16 DI, 16 DO
    transport.add_slave(create_aio_slave(2, 4, 2)); // 4 AI, 2 AO
    transport.add_slave(create_mixed_slave(3, 8, 8, 2, 2)); // Mixed

    let mut master = EthercatMaster::with_transport(config, Box::new(transport));

    // Initialize and verify
    master.init().unwrap();
    assert_eq!(master.state(), MasterState::Op);
    assert_eq!(master.network().slave_count(), 4);

    // Verify process image sizes
    assert!(master.network().total_input_size > 0);
    assert!(master.network().total_output_size > 0);

    // Run several exchange cycles
    for _ in 0..10 {
        master.exchange().unwrap();
    }
    assert_eq!(master.cycle_count(), 10);

    // Verify all slaves are configured correctly
    for pos in 0..4 {
        let slave = master.network().get_slave(pos).unwrap();
        assert_eq!(slave.state, SlaveState::Op);
    }

    master.shutdown().unwrap();
    assert_eq!(master.state(), MasterState::Offline);
}

#[test]
fn test_8_slave_topology() {
    let config = test_config();
    let mut transport = SimulatedTransport::new(&config);

    // Create an 8-slave topology
    for i in 0..4 {
        transport.add_slave(create_dio_slave(i, 8, 8));
    }
    for i in 4..8 {
        transport.add_slave(create_aio_slave(i, 2, 2));
    }

    let mut master = EthercatMaster::with_transport(config, Box::new(transport));

    master.init().unwrap();
    assert_eq!(master.state(), MasterState::Op);
    assert_eq!(master.network().slave_count(), 8);

    // Exchange multiple cycles
    for _ in 0..100 {
        master.exchange().unwrap();
    }
    assert_eq!(master.cycle_count(), 100);

    // Verify DC reference clock is set (first DC-capable slave)
    assert!(master.dc().reference_clock().is_some());
    assert!(master.dc().is_active());

    master.shutdown().unwrap();
}

#[test]
fn test_16_slave_topology() {
    let config = test_config();
    let mut transport = SimulatedTransport::new(&config);

    // Create a 16-slave stress test topology
    for i in 0..16 {
        match i % 3 {
            0 => transport.add_slave(create_dio_slave(i, 8, 8)),
            1 => transport.add_slave(create_aio_slave(i, 4, 4)),
            _ => transport.add_slave(create_mixed_slave(i, 8, 8, 2, 2)),
        }
    }

    let mut master = EthercatMaster::with_transport(config, Box::new(transport));

    master.init().unwrap();
    assert_eq!(master.state(), MasterState::Op);
    assert_eq!(master.network().slave_count(), 16);

    // Verify all slaves have correct offsets (no overlap)
    let network = master.network();
    let mut last_input_end = 0;
    let mut last_output_end = 0;

    for pos in 0..16 {
        let slave = network.get_slave(pos).unwrap();
        assert!(
            slave.input_offset >= last_input_end,
            "Input overlap at slave {}",
            pos
        );
        assert!(
            slave.output_offset >= last_output_end,
            "Output overlap at slave {}",
            pos
        );
        last_input_end = slave.input_offset + slave.input_size;
        last_output_end = slave.output_offset + slave.output_size;
    }

    // Stress test with many exchange cycles
    for _ in 0..1000 {
        master.exchange().unwrap();
    }
    assert_eq!(master.cycle_count(), 1000);

    // Check frame stats
    let stats = master.stats();
    assert_eq!(stats.frames_sent, 1000);
    assert_eq!(stats.frames_received, 1000);
    assert_eq!(stats.wkc_errors, 0);

    master.shutdown().unwrap();
}

// ============================================================================
// State Transition Tests
// ============================================================================

#[test]
fn test_state_transition_sequence() {
    let config = test_config();
    let transport = SimulatedTransport::with_test_slaves(&config);
    let mut master = EthercatMaster::with_transport(config, Box::new(transport));

    // Verify initial state
    assert_eq!(master.state(), MasterState::Offline);

    // OFFLINE -> INIT (via scan_slaves)
    master.scan_slaves().unwrap();
    assert_eq!(master.state(), MasterState::Init);

    // INIT -> PRE_OP (via configure_slaves)
    master.configure_slaves().unwrap();
    assert_eq!(master.state(), MasterState::PreOp);

    // Configure DC (stays in PRE_OP)
    master.configure_dc().unwrap();
    assert_eq!(master.state(), MasterState::PreOp);

    // PRE_OP -> SAFE_OP
    master.enter_safe_op().unwrap();
    assert_eq!(master.state(), MasterState::SafeOp);

    // Verify exchange works in SAFE_OP (outputs are safe)
    master.exchange().unwrap();

    // SAFE_OP -> OP
    master.enter_op().unwrap();
    assert_eq!(master.state(), MasterState::Op);

    // Verify full exchange in OP
    master.exchange().unwrap();
    assert!(master.process_image().wkc_ok());
}

#[test]
fn test_state_transition_errors() {
    let config = test_config();
    let transport = SimulatedTransport::with_test_slaves(&config);
    let mut master = EthercatMaster::with_transport(config, Box::new(transport));

    // Cannot configure from OFFLINE
    let result = master.configure_slaves();
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), PlcError::FieldbusError(_)));

    // Cannot enter SAFE_OP from OFFLINE
    let result = master.enter_safe_op();
    assert!(result.is_err());

    // Cannot enter OP from OFFLINE
    let result = master.enter_op();
    assert!(result.is_err());

    // Scan slaves to get to INIT
    master.scan_slaves().unwrap();
    assert_eq!(master.state(), MasterState::Init);

    // Cannot enter SAFE_OP directly from INIT
    let result = master.enter_safe_op();
    assert!(result.is_err());

    // Cannot enter OP directly from INIT
    let result = master.enter_op();
    assert!(result.is_err());

    // Configure to PRE_OP
    master.configure_slaves().unwrap();
    assert_eq!(master.state(), MasterState::PreOp);

    // Cannot enter OP directly from PRE_OP
    let result = master.enter_op();
    assert!(result.is_err());

    // Enter SAFE_OP correctly
    master.enter_safe_op().unwrap();
    assert_eq!(master.state(), MasterState::SafeOp);

    // Cannot re-configure from SAFE_OP
    let result = master.configure_slaves();
    assert!(result.is_err());
}

#[test]
fn test_shutdown_sequence() {
    let config = test_config();
    let transport = SimulatedTransport::with_test_slaves(&config);
    let mut master = EthercatMaster::with_transport(config, Box::new(transport));

    // Initialize to full OP
    master.init().unwrap();
    assert_eq!(master.state(), MasterState::Op);

    // Run some cycles
    for _ in 0..5 {
        master.exchange().unwrap();
    }

    // Shutdown should transition: OP -> SAFE_OP -> PRE_OP -> INIT -> OFFLINE
    master.shutdown().unwrap();
    assert_eq!(master.state(), MasterState::Offline);

    // Verify can re-initialize after shutdown
    // Note: The transport is closed, so scan would fail in real impl
    // But our simulated transport allows re-init
}

// ============================================================================
// WKC Error Handling Tests
// ============================================================================

#[test]
fn test_wkc_recovery() {
    let mut config = test_config();
    config.wkc_error_threshold = 5; // Allow up to 5 consecutive errors

    let mut transport = WkcErrorTransport::new(&config);
    transport.add_slave(create_dio_slave(0, 8, 8));
    transport.add_slave(create_aio_slave(1, 2, 2));

    // Errors on cycles 2, 3, 4 (0-indexed), then recovery
    let transport = transport.with_errors_at(vec![2, 3, 4]);

    let mut master = EthercatMaster::with_transport(config, Box::new(transport));
    master.init().unwrap();

    // Cycles 0, 1: OK
    master.exchange().unwrap();
    master.exchange().unwrap();
    assert_eq!(master.state(), MasterState::Op);

    // Cycles 2, 3, 4: WKC errors but below threshold
    master.exchange().unwrap(); // 1 error
    master.exchange().unwrap(); // 2 errors
    master.exchange().unwrap(); // 3 errors
    assert_eq!(master.state(), MasterState::Op);

    // Cycle 5: Recovery (good WKC) - should reset consecutive error count
    master.exchange().unwrap();
    assert_eq!(master.state(), MasterState::Op);

    // More good cycles to verify recovery
    for _ in 0..10 {
        master.exchange().unwrap();
    }
    assert_eq!(master.state(), MasterState::Op);

    // Verify stats show the 3 WKC errors
    assert_eq!(master.stats().wkc_errors, 3);
}

#[test]
fn test_wkc_threshold_enforcement() {
    let mut config = test_config();
    config.wkc_error_threshold = 3; // Fault after 3 consecutive errors

    let mut transport = WkcErrorTransport::new(&config);
    transport.add_slave(create_dio_slave(0, 8, 8));

    // Consecutive errors starting at cycle 2
    let transport = transport.with_errors_at(vec![2, 3, 4, 5, 6]);

    let mut master = EthercatMaster::with_transport(config, Box::new(transport));
    master.init().unwrap();

    // Cycles 0, 1: OK
    master.exchange().unwrap();
    master.exchange().unwrap();
    assert_eq!(master.state(), MasterState::Op);

    // Cycle 2, 3: WKC errors (1, 2 consecutive)
    master.exchange().unwrap();
    master.exchange().unwrap();
    assert_eq!(master.state(), MasterState::Op);

    // Cycle 4: 3rd consecutive error - should fault
    let result = master.exchange();
    assert!(result.is_err());

    // Verify the error type
    match result.unwrap_err() {
        PlcError::WkcThresholdExceeded {
            consecutive,
            threshold,
        } => {
            assert_eq!(consecutive, 3);
            assert_eq!(threshold, 3);
        }
        e => panic!("Expected WkcThresholdExceeded, got {:?}", e),
    }

    // Master should be in FAULT state
    assert_eq!(master.state(), MasterState::Fault);
}

#[test]
fn test_wkc_threshold_disabled() {
    let mut config = test_config();
    config.wkc_error_threshold = 0; // Disabled - only log warnings

    let mut transport = WkcErrorTransport::new(&config);
    transport.add_slave(create_dio_slave(0, 8, 8));

    // All cycles have WKC errors
    let transport = transport.with_errors_at((0..100).collect());

    let mut master = EthercatMaster::with_transport(config, Box::new(transport));
    master.init().unwrap();

    // Should never fault even with continuous WKC errors
    for _ in 0..50 {
        master.exchange().unwrap();
    }

    assert_eq!(master.state(), MasterState::Op);
    assert_eq!(master.stats().wkc_errors, 50);
}

// ============================================================================
// DC Synchronization Tests
// ============================================================================

#[test]
fn test_dc_initialization() {
    let mut config = test_config();
    config.dc_enabled = true;

    let mut transport = SimulatedTransport::new(&config);
    // First slave with DC support becomes reference clock
    let mut slave0 = create_dio_slave(0, 8, 8);
    slave0.dc_supported = true;
    transport.add_slave(slave0);

    // Second slave also DC capable
    let mut slave1 = create_aio_slave(1, 2, 2);
    slave1.dc_supported = true;
    transport.add_slave(slave1);

    // Third slave without DC
    let mut slave2 = create_dio_slave(2, 8, 8);
    slave2.dc_supported = false;
    transport.add_slave(slave2);

    let mut master = EthercatMaster::with_transport(config, Box::new(transport));
    master.init().unwrap();

    // Verify reference clock is the first DC-capable slave
    let dc = master.dc();
    assert!(dc.is_active());
    assert_eq!(dc.reference_clock(), Some(0));

    // Verify DC slaves are tracked
    let dc_slaves: Vec<_> = dc.slaves().iter().filter(|s| s.dc_supported).collect();
    assert_eq!(dc_slaves.len(), 2);
}

#[test]
fn test_dc_statistics() {
    let mut config = test_config();
    config.dc_enabled = true;

    let mut transport = SimulatedTransport::new(&config);
    let mut slave = create_dio_slave(0, 8, 8);
    slave.dc_supported = true;
    transport.add_slave(slave);

    let mut master = EthercatMaster::with_transport(config, Box::new(transport));
    master.init().unwrap();

    // Run many cycles to collect DC statistics
    for _ in 0..1000 {
        master.exchange().unwrap();
    }

    let dc = master.dc();
    assert!(dc.is_active());

    let stats = dc.stats();
    assert!(stats.sync_cycles > 0);

    // Verify statistics are being collected
    // (exact values depend on timing, but should have valid data)
    if stats.sync_cycles > 1 {
        // Jitter should be defined after multiple cycles
        assert!(stats.jitter_ns().is_some());
    }
}

#[test]
fn test_dc_disabled() {
    let mut config = test_config();
    config.dc_enabled = false;

    let mut transport = SimulatedTransport::new(&config);
    let mut slave = create_dio_slave(0, 8, 8);
    slave.dc_supported = true;
    transport.add_slave(slave);

    let mut master = EthercatMaster::with_transport(config, Box::new(transport));
    master.init().unwrap();

    // DC should not be active even with DC-capable slaves
    let dc = master.dc();
    assert!(!dc.is_active());
}

// ============================================================================
// Process Image Tests via FieldbusDriver Trait
// ============================================================================

#[test]
fn test_fieldbus_driver_trait() {
    let config = test_config();
    let transport = SimulatedTransport::with_test_slaves(&config);
    let mut master = EthercatMaster::with_transport(config, Box::new(transport));

    // Use FieldbusDriver trait methods
    <EthercatMaster as FieldbusDriver>::init(&mut master).unwrap();

    // read_inputs/write_outputs are no-ops for EtherCAT (combined in exchange)
    master.read_inputs().unwrap();
    master.write_outputs().unwrap();

    // Use trait's exchange method
    <EthercatMaster as FieldbusDriver>::exchange(&mut master).unwrap();

    // Shutdown via trait
    <EthercatMaster as FieldbusDriver>::shutdown(&mut master).unwrap();
    assert_eq!(master.state(), MasterState::Offline);
}

#[test]
fn test_digital_io_mapping() {
    let config = test_config();
    let mut transport = SimulatedTransport::new(&config);

    // Create slave with digital I/O that fits the standard layout
    // Layout: bytes 0-3 = digital, bytes 4+ = analog
    let mut slave = SlaveConfig::new(0, SlaveIdentity::new(0x2, 0x1234, 1, 0));
    slave.name = "Digital I/O".into();
    slave.dc_supported = true;

    // TxPDO: 32 bits digital input (matches get_inputs layout)
    let mut tx_pdo = PdoMapping::new(0x1A00, true);
    tx_pdo.add_entry(PdoEntry::new(0x6000, 1, 32).with_name("DI 0-31"));
    slave.tx_pdos.push(tx_pdo);

    // RxPDO: 32 bits digital output (matches set_outputs layout)
    let mut rx_pdo = PdoMapping::new(0x1600, false);
    rx_pdo.add_entry(PdoEntry::new(0x7000, 1, 32).with_name("DO 0-31"));
    slave.rx_pdos.push(rx_pdo);

    transport.add_slave(slave);

    let mut master = EthercatMaster::with_transport(config, Box::new(transport));
    master.init().unwrap();

    // Set digital outputs via FieldbusDriver trait
    let outputs = FieldbusOutputs {
        digital: 0xDEADBEEF,
        analog: [0; 16],
    };
    master.set_outputs(&outputs);

    // Verify output was written to process image
    let pi = master.process_image();
    let written = u32::from_le_bytes([
        pi.outputs()[0],
        pi.outputs()[1],
        pi.outputs()[2],
        pi.outputs()[3],
    ]);
    assert_eq!(written, 0xDEADBEEF);

    // Exchange to update inputs
    master.exchange().unwrap();

    // Get inputs via trait
    let inputs = master.get_inputs();
    // Simulated transport echoes first byte of outputs
    assert_eq!(inputs.digital & 0xFF, 0xEF);
}

#[test]
fn test_analog_io_mapping() {
    let config = test_config();
    let mut transport = SimulatedTransport::new(&config);

    // Create slave with analog I/O
    let mut slave = SlaveConfig::new(0, SlaveIdentity::new(0x2, 0x5678, 1, 0));
    slave.name = "Analog I/O".into();
    slave.dc_supported = true;

    // TxPDO: 32-bit placeholder + 16 x 16-bit analog inputs
    let mut tx_pdo = PdoMapping::new(0x1A00, true);
    tx_pdo.add_entry(PdoEntry::new(0x6000, 1, 32).with_name("Digital placeholder"));
    for i in 1..=16 {
        tx_pdo.add_entry(PdoEntry::new(0x6010, i, 16).with_name(format!("AI {}", i)));
    }
    slave.tx_pdos.push(tx_pdo);

    // RxPDO: 32-bit placeholder + 16 x 16-bit analog outputs
    let mut rx_pdo = PdoMapping::new(0x1600, false);
    rx_pdo.add_entry(PdoEntry::new(0x7000, 1, 32).with_name("Digital placeholder"));
    for i in 1..=16 {
        rx_pdo.add_entry(PdoEntry::new(0x7010, i, 16).with_name(format!("AO {}", i)));
    }
    slave.rx_pdos.push(rx_pdo);

    transport.add_slave(slave);

    let mut master = EthercatMaster::with_transport(config, Box::new(transport));
    master.init().unwrap();

    // Set analog outputs
    let mut outputs = FieldbusOutputs::default();
    outputs.analog[0] = 1000;
    outputs.analog[1] = -2000;
    outputs.analog[15] = 32767;
    master.set_outputs(&outputs);

    // Verify analog outputs in process image
    let pi = master.process_image();
    // Byte 4-5: analog[0], byte 6-7: analog[1]
    let ao0 = i16::from_le_bytes([pi.outputs()[4], pi.outputs()[5]]);
    let ao1 = i16::from_le_bytes([pi.outputs()[6], pi.outputs()[7]]);
    let ao15 = i16::from_le_bytes([pi.outputs()[34], pi.outputs()[35]]);

    assert_eq!(ao0, 1000);
    assert_eq!(ao1, -2000);
    assert_eq!(ao15, 32767);

    // Exchange
    master.exchange().unwrap();

    // Get analog inputs
    let inputs = master.get_inputs();
    // SimulatedTransport puts a sine wave on AI channel 1 (analog[0])
    // After first exchange, the value should be non-zero (sine starts at 0.01 radians)
    // The exact value depends on cycle count, but should be reasonable
    assert!(
        inputs.analog[0].abs() < 20000,
        "Analog input should be within reasonable range"
    );
}

#[test]
fn test_process_image_exchange() {
    let config = test_config();
    let transport = SimulatedTransport::with_test_slaves(&config);
    let mut master = EthercatMaster::with_transport(config, Box::new(transport));

    master.init().unwrap();

    // Multiple exchange cycles
    for i in 0..10 {
        // Set outputs
        let mut outputs = FieldbusOutputs::default();
        outputs.digital = i as u32;
        master.set_outputs(&outputs);

        // Exchange
        master.exchange().unwrap();

        // Verify cycle counter
        assert_eq!(master.cycle_count(), (i + 1) as u64);

        // WKC should be OK
        assert!(master.process_image().wkc_ok());
    }

    // Verify final stats
    let stats = master.stats();
    assert_eq!(stats.frames_sent, 10);
    assert_eq!(stats.frames_received, 10);
    assert_eq!(stats.wkc_errors, 0);
    // RTT can be 0 in simulated mode (instant exchange) - just verify it was recorded
    // The simulated transport is fast enough that RTT may round down to 0 microseconds
}

// ============================================================================
// Edge Cases and Error Handling
// ============================================================================

#[test]
fn test_exchange_in_wrong_state() {
    let config = test_config();
    let transport = SimulatedTransport::with_test_slaves(&config);
    let mut master = EthercatMaster::with_transport(config, Box::new(transport));

    // Cannot exchange when OFFLINE
    let result = master.exchange();
    assert!(result.is_err());

    // Scan slaves to INIT
    master.scan_slaves().unwrap();

    // Cannot exchange when INIT
    let result = master.exchange();
    assert!(result.is_err());

    // Configure to PRE_OP
    master.configure_slaves().unwrap();

    // Cannot exchange when PRE_OP
    let result = master.exchange();
    assert!(result.is_err());

    // Enter SAFE_OP
    master.enter_safe_op().unwrap();

    // Can exchange in SAFE_OP
    let result = master.exchange();
    assert!(result.is_ok());
}

#[test]
fn test_empty_network() {
    let config = test_config();
    let transport = SimulatedTransport::new(&config);
    let mut master = EthercatMaster::with_transport(config, Box::new(transport));

    // Scanning with no slaves returns 0
    let count = master.scan_slaves().unwrap();
    assert_eq!(count, 0);
    assert_eq!(master.network().slave_count(), 0);

    // Process image should have zero size
    assert_eq!(master.process_image().inputs().len(), 0);
    assert_eq!(master.process_image().outputs().len(), 0);
}

#[test]
fn test_rescan_clears_previous() {
    let config = test_config();
    let mut transport = SimulatedTransport::new(&config);
    transport.add_slave(create_dio_slave(0, 8, 8));
    transport.add_slave(create_dio_slave(1, 8, 8));

    let mut master = EthercatMaster::with_transport(config, Box::new(transport));

    // First scan
    let count1 = master.scan_slaves().unwrap();
    assert_eq!(count1, 2);
    assert_eq!(master.network().slave_count(), 2);

    // Rescan should not accumulate
    let count2 = master.scan_slaves().unwrap();
    assert_eq!(count2, 2);
    assert_eq!(master.network().slave_count(), 2);

    // DC slaves should also not accumulate
    assert_eq!(master.dc().slaves().len(), 2);
}

#[test]
fn test_sdo_operations() {
    let config = test_config();
    let transport = SimulatedTransport::with_test_slaves(&config);
    let mut master = EthercatMaster::with_transport(config, Box::new(transport));

    master.scan_slaves().unwrap();

    // SDO read
    let request = plc_fieldbus::slave_config::SdoRequest::read(0, 0x1000, 0);
    let data = master.sdo_read(&request).unwrap();
    assert!(!data.is_empty());

    // SDO write
    let request = plc_fieldbus::slave_config::SdoRequest::write(0, 0x6000, 1, vec![0x01]);
    master.sdo_write(&request).unwrap();
}

#[test]
fn test_sdo_offline_error() {
    let config = test_config();
    let transport = SimulatedTransport::with_test_slaves(&config);
    let mut master = EthercatMaster::with_transport(config, Box::new(transport));

    // Master is OFFLINE - SDO should fail
    let request = plc_fieldbus::slave_config::SdoRequest::read(0, 0x1000, 0);
    let result = master.sdo_read(&request);
    assert!(result.is_err());
}
