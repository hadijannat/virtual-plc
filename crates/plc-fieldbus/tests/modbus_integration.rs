//! Modbus TCP integration tests using `MockModbusServer`.
//!
//! These tests verify the `ModbusTcpDriver` behavior against a real TCP connection
//! with a mock server providing controllable fault injection.
//!
//! # Test Categories
//!
//! - **Happy path tests**: Verify correct operation for all supported function codes
//! - **Exception tests**: Verify proper handling of Modbus exception responses
//! - **Timeout/reconnection tests**: Verify connection recovery behavior
//! - **Edge cases**: Verify handling of protocol anomalies

mod mock_modbus_server;

use mock_modbus_server::{MockBehavior, MockModbusServer};
use plc_fieldbus::{
    FieldbusDriver, FieldbusOutputs, FunctionCode, ModbusMapping, ModbusTcpConfig, ModbusTcpDriver,
};
use std::time::Duration;

// ============================================================================
// Test Helpers
// ============================================================================

/// Create a driver config pointing to the mock server.
fn config_for_server(server: &MockModbusServer) -> ModbusTcpConfig {
    ModbusTcpConfig {
        server_addr: server.local_addr(),
        unit_id: 1,
        connect_timeout: Duration::from_secs(2),
        io_timeout: Duration::from_millis(500),
        max_reconnect_attempts: 3,
        reconnect_delay: Duration::from_millis(100),
        digital_input_mapping: Some(ModbusMapping {
            address: 0,
            quantity: 32,
            function: FunctionCode::ReadCoils,
        }),
        digital_output_mapping: Some(ModbusMapping {
            address: 0,
            quantity: 32,
            function: FunctionCode::WriteMultipleCoils,
        }),
        analog_input_mapping: Some(ModbusMapping {
            address: 0,
            quantity: 16,
            function: FunctionCode::ReadHoldingRegisters,
        }),
        analog_output_mapping: Some(ModbusMapping {
            address: 0,
            quantity: 16,
            function: FunctionCode::WriteMultipleRegisters,
        }),
    }
}

/// Create a driver config with discrete inputs instead of coils for digital input.
fn config_with_discrete_inputs(server: &MockModbusServer) -> ModbusTcpConfig {
    let mut config = config_for_server(server);
    config.digital_input_mapping = Some(ModbusMapping {
        address: 0,
        quantity: 32,
        function: FunctionCode::ReadDiscreteInputs,
    });
    config
}

/// Create a driver config using input registers for analog inputs.
fn config_with_input_registers(server: &MockModbusServer) -> ModbusTcpConfig {
    let mut config = config_for_server(server);
    config.analog_input_mapping = Some(ModbusMapping {
        address: 0,
        quantity: 16,
        function: FunctionCode::ReadInputRegisters,
    });
    config
}

// ============================================================================
// Happy Path Tests
// ============================================================================

#[test]
fn test_read_coils_happy_path() {
    let server = MockModbusServer::start(MockBehavior::Normal).unwrap();

    // Set up 32 coils with known pattern
    // Pattern: bits 0, 2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 22, 24, 26, 28, 30 = ON
    for i in 0..32 {
        server.set_coil(i, i % 2 == 0);
    }

    let config = config_for_server(&server);
    let mut driver = ModbusTcpDriver::with_config(config);

    driver.init().unwrap();
    assert!(driver.is_operational());

    driver.read_inputs().unwrap();
    let inputs = driver.get_inputs();

    // Verify pattern: every even bit should be 1
    // Expected: 0b01010101_01010101_01010101_01010101 = 0x55555555
    assert_eq!(
        inputs.digital, 0x55555555,
        "Expected coil pattern 0x55555555, got 0x{:08X}",
        inputs.digital
    );

    driver.shutdown().unwrap();
    server.stop();
}

#[test]
fn test_read_discrete_inputs_happy_path() {
    let server = MockModbusServer::start(MockBehavior::Normal).unwrap();

    // Set up 32 discrete inputs with alternating pattern (odd bits ON)
    for i in 0..32 {
        server.set_discrete_input(i, i % 2 == 1);
    }

    let config = config_with_discrete_inputs(&server);
    let mut driver = ModbusTcpDriver::with_config(config);

    driver.init().unwrap();
    driver.read_inputs().unwrap();
    let inputs = driver.get_inputs();

    // Expected: 0b10101010_10101010_10101010_10101010 = 0xAAAAAAAA
    assert_eq!(
        inputs.digital, 0xAAAAAAAA,
        "Expected discrete input pattern 0xAAAAAAAA, got 0x{:08X}",
        inputs.digital
    );

    driver.shutdown().unwrap();
    server.stop();
}

#[test]
fn test_read_holding_registers_happy_path() {
    let server = MockModbusServer::start(MockBehavior::Normal).unwrap();

    // Set up 16 holding registers with incrementing values
    for i in 0..16 {
        server.set_holding_register(i, (i * 1000) as u16);
    }

    let config = config_for_server(&server);
    let mut driver = ModbusTcpDriver::with_config(config);

    driver.init().unwrap();
    driver.read_inputs().unwrap();
    let inputs = driver.get_inputs();

    // Verify each analog input channel
    for i in 0..16 {
        let expected = (i * 1000) as i16;
        assert_eq!(
            inputs.analog[i], expected,
            "Analog input {} mismatch: expected {}, got {}",
            i, expected, inputs.analog[i]
        );
    }

    driver.shutdown().unwrap();
    server.stop();
}

#[test]
fn test_read_input_registers_happy_path() {
    let server = MockModbusServer::start(MockBehavior::Normal).unwrap();

    // Set up 16 input registers with specific values
    for i in 0..16 {
        server.set_input_register(i, 0x1000 + i as u16);
    }

    let config = config_with_input_registers(&server);
    let mut driver = ModbusTcpDriver::with_config(config);

    driver.init().unwrap();
    driver.read_inputs().unwrap();
    let inputs = driver.get_inputs();

    // Verify each analog input channel
    for i in 0..16 {
        let expected = (0x1000 + i) as i16;
        assert_eq!(
            inputs.analog[i], expected,
            "Analog input {} mismatch: expected 0x{:04X}, got 0x{:04X}",
            i, expected, inputs.analog[i]
        );
    }

    driver.shutdown().unwrap();
    server.stop();
}

#[test]
fn test_write_single_coil() {
    let server = MockModbusServer::start(MockBehavior::Normal).unwrap();

    // Configure for single coil output
    let mut config = config_for_server(&server);
    config.digital_output_mapping = Some(ModbusMapping {
        address: 10,
        quantity: 1,
        function: FunctionCode::WriteMultipleCoils,
    });
    config.analog_output_mapping = None;

    let mut driver = ModbusTcpDriver::with_config(config);
    driver.init().unwrap();

    // Set output (bit 0 = ON)
    let mut outputs = FieldbusOutputs::default();
    outputs.digital = 0x0000_0001;
    driver.set_outputs(&outputs);

    driver.write_outputs().unwrap();

    // Verify the coil was set on the server
    // Note: write_coils with single value uses FC 0x05 (WriteSingleCoil)
    assert!(
        server.get_coil(10),
        "Coil 10 should be ON after write"
    );

    driver.shutdown().unwrap();
    server.stop();
}

#[test]
fn test_write_single_register() {
    let server = MockModbusServer::start(MockBehavior::Normal).unwrap();

    // Configure for single register output
    let mut config = config_for_server(&server);
    config.digital_output_mapping = None;
    config.analog_output_mapping = Some(ModbusMapping {
        address: 5,
        quantity: 1,
        function: FunctionCode::WriteMultipleRegisters,
    });

    let mut driver = ModbusTcpDriver::with_config(config);
    driver.init().unwrap();

    // Set output value
    let mut outputs = FieldbusOutputs::default();
    outputs.analog[0] = 0x1234;
    driver.set_outputs(&outputs);

    driver.write_outputs().unwrap();

    // Verify the register was written
    // Note: write_registers with single value uses FC 0x06 (WriteSingleRegister)
    assert_eq!(
        server.get_holding_register(5),
        0x1234,
        "Holding register 5 should contain 0x1234"
    );

    driver.shutdown().unwrap();
    server.stop();
}

#[test]
fn test_write_multiple_coils() {
    let server = MockModbusServer::start(MockBehavior::Normal).unwrap();

    let config = config_for_server(&server);
    let mut driver = ModbusTcpDriver::with_config(config);

    driver.init().unwrap();

    // Set 32 coils with pattern 0xDEADBEEF
    let mut outputs = FieldbusOutputs::default();
    outputs.digital = 0xDEADBEEF;
    driver.set_outputs(&outputs);

    driver.write_outputs().unwrap();

    // Verify coils on server
    // 0xDEADBEEF = 11011110_10101101_10111110_11101111
    let mut actual_pattern = 0u32;
    for i in 0..32 {
        if server.get_coil(i) {
            actual_pattern |= 1 << i;
        }
    }

    assert_eq!(
        actual_pattern, 0xDEADBEEF,
        "Expected coil pattern 0xDEADBEEF, got 0x{:08X}",
        actual_pattern
    );

    driver.shutdown().unwrap();
    server.stop();
}

#[test]
fn test_write_multiple_registers() {
    let server = MockModbusServer::start(MockBehavior::Normal).unwrap();

    let config = config_for_server(&server);
    let mut driver = ModbusTcpDriver::with_config(config);

    driver.init().unwrap();

    // Set 16 analog outputs with specific values
    let mut outputs = FieldbusOutputs::default();
    for i in 0..16 {
        outputs.analog[i] = (i * 100 + 50) as i16;
    }
    driver.set_outputs(&outputs);

    driver.write_outputs().unwrap();

    // Verify registers on server
    for i in 0..16 {
        let expected = (i * 100 + 50) as u16;
        let actual = server.get_holding_register(i);
        assert_eq!(
            actual, expected,
            "Register {} mismatch: expected {}, got {}",
            i, expected, actual
        );
    }

    driver.shutdown().unwrap();
    server.stop();
}

// ============================================================================
// Exception Tests
// ============================================================================

#[test]
fn test_exception_illegal_function() {
    let server = MockModbusServer::start(MockBehavior::Exception(0x01)).unwrap();

    let config = config_for_server(&server);
    let mut driver = ModbusTcpDriver::with_config(config);

    driver.init().unwrap();

    let result = driver.read_inputs();
    assert!(result.is_err(), "Expected error for illegal function exception");

    let err = result.unwrap_err();
    let err_msg = format!("{}", err);
    assert!(
        err_msg.contains("Illegal Function"),
        "Expected 'Illegal Function' in error message, got: {}",
        err_msg
    );

    driver.shutdown().unwrap();
    server.stop();
}

#[test]
fn test_exception_illegal_address() {
    let server = MockModbusServer::start(MockBehavior::Exception(0x02)).unwrap();

    let config = config_for_server(&server);
    let mut driver = ModbusTcpDriver::with_config(config);

    driver.init().unwrap();

    let result = driver.read_inputs();
    assert!(result.is_err(), "Expected error for illegal address exception");

    let err = result.unwrap_err();
    let err_msg = format!("{}", err);
    assert!(
        err_msg.contains("Illegal Data Address"),
        "Expected 'Illegal Data Address' in error message, got: {}",
        err_msg
    );

    driver.shutdown().unwrap();
    server.stop();
}

#[test]
fn test_exception_illegal_value() {
    let server = MockModbusServer::start(MockBehavior::Exception(0x03)).unwrap();

    let config = config_for_server(&server);
    let mut driver = ModbusTcpDriver::with_config(config);

    driver.init().unwrap();

    let result = driver.read_inputs();
    assert!(result.is_err(), "Expected error for illegal value exception");

    let err = result.unwrap_err();
    let err_msg = format!("{}", err);
    assert!(
        err_msg.contains("Illegal Data Value"),
        "Expected 'Illegal Data Value' in error message, got: {}",
        err_msg
    );

    driver.shutdown().unwrap();
    server.stop();
}

#[test]
fn test_exception_server_failure() {
    let server = MockModbusServer::start(MockBehavior::Exception(0x04)).unwrap();

    let config = config_for_server(&server);
    let mut driver = ModbusTcpDriver::with_config(config);

    driver.init().unwrap();

    let result = driver.read_inputs();
    assert!(result.is_err(), "Expected error for server failure exception");

    let err = result.unwrap_err();
    let err_msg = format!("{}", err);
    assert!(
        err_msg.contains("Server Device Failure"),
        "Expected 'Server Device Failure' in error message, got: {}",
        err_msg
    );

    driver.shutdown().unwrap();
    server.stop();
}

// ============================================================================
// Timeout and Reconnection Tests
// ============================================================================

#[test]
fn test_connect_timeout() {
    // Use a non-routable IP to simulate connection timeout
    // 10.255.255.1 is a non-routable address in the reserved private range
    let config = ModbusTcpConfig {
        server_addr: "10.255.255.1:502".parse().unwrap(),
        unit_id: 1,
        connect_timeout: Duration::from_millis(100), // Short timeout
        io_timeout: Duration::from_millis(100),
        max_reconnect_attempts: 1,
        reconnect_delay: Duration::from_millis(10),
        digital_input_mapping: None,
        digital_output_mapping: None,
        analog_input_mapping: None,
        analog_output_mapping: None,
    };

    let mut driver = ModbusTcpDriver::with_config(config);

    let result = driver.init();
    assert!(result.is_err(), "Expected connection timeout error");

    let err = result.unwrap_err();
    let err_msg = format!("{}", err);
    assert!(
        err_msg.contains("Connection failed") || err_msg.contains("timed out"),
        "Expected connection/timeout error, got: {}",
        err_msg
    );
}

#[test]
fn test_read_timeout() {
    // Server accepts connection but delays response longer than timeout
    let server = MockModbusServer::start(MockBehavior::DelayMs(2000)).unwrap();

    let mut config = config_for_server(&server);
    config.io_timeout = Duration::from_millis(100); // 100ms timeout, server delays 2000ms

    let mut driver = ModbusTcpDriver::with_config(config);
    driver.init().unwrap();

    let result = driver.read_inputs();
    assert!(result.is_err(), "Expected read timeout error");

    // The connection should be marked as disconnected after timeout
    assert!(
        !driver.is_operational(),
        "Driver should not be operational after timeout"
    );

    server.stop();
}

#[test]
fn test_reconnect_success() {
    let server = MockModbusServer::start(MockBehavior::Normal).unwrap();

    let config = config_for_server(&server);
    let mut driver = ModbusTcpDriver::with_config(config);

    driver.init().unwrap();
    assert!(driver.is_operational());

    // Perform a successful read
    driver.read_inputs().unwrap();

    // Simulate connection drop by setting behavior to drop
    server.set_behavior(MockBehavior::DropConnection);

    // This should fail and trigger reconnection state
    let _ = driver.read_inputs();

    // Reset server to normal behavior
    server.set_behavior(MockBehavior::Normal);

    // Give the driver time to reconnect and try again
    // The driver uses non-blocking reconnection, so we need to call read_inputs
    // multiple times to allow reconnection attempts
    let mut success = false;
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(150));
        if driver.read_inputs().is_ok() {
            success = true;
            break;
        }
    }

    assert!(success, "Driver should reconnect successfully");
    assert!(driver.is_operational());

    driver.shutdown().unwrap();
    server.stop();
}

#[test]
fn test_reconnect_max_attempts() {
    // Start server, then stop it to simulate permanent failure
    let server = MockModbusServer::start(MockBehavior::Normal).unwrap();
    let server_addr = server.local_addr();

    let config = ModbusTcpConfig {
        server_addr,
        unit_id: 1,
        connect_timeout: Duration::from_millis(100),
        io_timeout: Duration::from_millis(100),
        max_reconnect_attempts: 3,
        reconnect_delay: Duration::from_millis(50),
        digital_input_mapping: Some(ModbusMapping {
            address: 0,
            quantity: 8,
            function: FunctionCode::ReadCoils,
        }),
        digital_output_mapping: None,
        analog_input_mapping: None,
        analog_output_mapping: None,
    };

    let mut driver = ModbusTcpDriver::with_config(config.clone());
    driver.init().unwrap();

    // Stop the server to simulate failure
    server.stop();

    // Wait a moment for the server to fully stop
    std::thread::sleep(Duration::from_millis(50));

    // Now reads should fail and trigger reconnection attempts
    // After max_reconnect_attempts, driver should enter Failed state
    let mut max_attempts_exceeded = false;
    for _ in 0..20 {
        let result = driver.read_inputs();
        if let Err(e) = result {
            let msg = format!("{}", e);
            if msg.contains("Max reconnection attempts") {
                max_attempts_exceeded = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    assert!(
        max_attempts_exceeded,
        "Expected max reconnection attempts exceeded error"
    );
    assert!(
        !driver.is_operational(),
        "Driver should not be operational after max reconnect attempts"
    );
}

// ============================================================================
// Edge Case Tests
// ============================================================================

#[test]
fn test_wrong_unit_id_response() {
    let server = MockModbusServer::start(MockBehavior::WrongUnitId).unwrap();

    let config = config_for_server(&server);
    let mut driver = ModbusTcpDriver::with_config(config);

    driver.init().unwrap();

    let result = driver.read_inputs();
    assert!(result.is_err(), "Expected error for wrong unit ID");

    let err = result.unwrap_err();
    let err_msg = format!("{}", err);
    assert!(
        err_msg.contains("Unit ID mismatch"),
        "Expected 'Unit ID mismatch' in error message, got: {}",
        err_msg
    );

    server.stop();
}

#[test]
fn test_wrong_transaction_id_response() {
    let server = MockModbusServer::start(MockBehavior::WrongTransactionId).unwrap();

    let config = config_for_server(&server);
    let mut driver = ModbusTcpDriver::with_config(config);

    driver.init().unwrap();

    let result = driver.read_inputs();
    assert!(result.is_err(), "Expected error for wrong transaction ID");

    let err = result.unwrap_err();
    let err_msg = format!("{}", err);
    assert!(
        err_msg.contains("Transaction ID mismatch"),
        "Expected 'Transaction ID mismatch' in error message, got: {}",
        err_msg
    );

    server.stop();
}

#[test]
fn test_oversized_response() {
    // The mock server's CorruptResponse behavior sends garbage data
    // which effectively tests malformed/oversized response handling
    let server = MockModbusServer::start(MockBehavior::CorruptResponse).unwrap();

    let config = config_for_server(&server);
    let mut driver = ModbusTcpDriver::with_config(config);

    driver.init().unwrap();

    let result = driver.read_inputs();
    assert!(
        result.is_err(),
        "Expected error for corrupted/oversized response"
    );

    // The error could be various things depending on what garbage was received
    // Just verify we get an error and don't panic

    server.stop();
}

// ============================================================================
// Full Cycle Integration Tests
// ============================================================================

#[test]
fn test_full_exchange_cycle() {
    let server = MockModbusServer::start(MockBehavior::Normal).unwrap();

    // Set up initial server state
    for i in 0..32 {
        server.set_coil(i, i < 16); // First 16 coils ON
    }
    for i in 0..16 {
        server.set_holding_register(i, (1000 + i) as u16);
    }

    let config = config_for_server(&server);
    let mut driver = ModbusTcpDriver::with_config(config);

    driver.init().unwrap();

    // Read inputs
    driver.read_inputs().unwrap();
    let inputs = driver.get_inputs();

    // Verify digital inputs (first 16 bits ON)
    assert_eq!(
        inputs.digital, 0x0000FFFF,
        "Expected digital inputs 0x0000FFFF"
    );

    // Verify analog inputs
    for i in 0..16 {
        assert_eq!(inputs.analog[i], (1000 + i) as i16);
    }

    // Set outputs
    let mut outputs = FieldbusOutputs::default();
    outputs.digital = 0xCAFEBABE;
    for i in 0..16 {
        outputs.analog[i] = (2000 + i) as i16;
    }
    driver.set_outputs(&outputs);

    // Write outputs
    driver.write_outputs().unwrap();

    // Verify outputs were written to server
    let mut coil_pattern = 0u32;
    for i in 0..32 {
        if server.get_coil(i) {
            coil_pattern |= 1 << i;
        }
    }
    assert_eq!(coil_pattern, 0xCAFEBABE);

    for i in 0..16 {
        assert_eq!(server.get_holding_register(i), (2000 + i) as u16);
    }

    // Full exchange via trait
    driver.exchange().unwrap();

    driver.shutdown().unwrap();
    assert!(!driver.is_operational());

    server.stop();
}

#[test]
fn test_multiple_exchange_cycles() {
    let server = MockModbusServer::start(MockBehavior::Normal).unwrap();

    let config = config_for_server(&server);
    let mut driver = ModbusTcpDriver::with_config(config);

    driver.init().unwrap();

    // Run 100 exchange cycles
    for cycle in 0..100 {
        // Update server state
        server.set_holding_register(0, cycle as u16);

        // Set outputs based on cycle
        let mut outputs = FieldbusOutputs::default();
        outputs.digital = cycle;
        outputs.analog[0] = cycle as i16;
        driver.set_outputs(&outputs);

        // Full exchange
        driver.exchange().unwrap();

        // Verify inputs
        let inputs = driver.get_inputs();
        assert_eq!(inputs.analog[0], cycle as i16);

        // Verify outputs were written
        assert_eq!(server.get_holding_register(0), cycle as u16);
    }

    driver.shutdown().unwrap();
    server.stop();
}

#[test]
fn test_runtime_behavior_change() {
    let server = MockModbusServer::start(MockBehavior::Normal).unwrap();

    let config = config_for_server(&server);
    let mut driver = ModbusTcpDriver::with_config(config);

    driver.init().unwrap();

    // First exchange should succeed
    driver.exchange().unwrap();

    // Change server to return exception
    server.set_behavior(MockBehavior::Exception(0x02));

    // Next exchange should fail
    let result = driver.exchange();
    assert!(result.is_err());

    // Change back to normal
    server.set_behavior(MockBehavior::Normal);

    // Need to reconnect since connection may be dropped
    // Try exchanges until one succeeds (reconnection)
    let mut recovered = false;
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(150));
        if driver.exchange().is_ok() {
            recovered = true;
            break;
        }
    }
    assert!(recovered, "Driver should recover after server returns to normal");

    driver.shutdown().unwrap();
    server.stop();
}
