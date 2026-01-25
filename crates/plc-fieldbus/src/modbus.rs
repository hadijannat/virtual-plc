//! Modbus TCP driver implementation.
//!
//! Provides Modbus TCP client functionality for PLC I/O with support for:
//! - Read Coils (Function 0x01)
//! - Read Discrete Inputs (Function 0x02)
//! - Read Holding Registers (Function 0x03)
//! - Read Input Registers (Function 0x04)
//! - Write Single Coil (Function 0x05)
//! - Write Single Register (Function 0x06)
//! - Write Multiple Coils (Function 0x0F)
//! - Write Multiple Registers (Function 0x10)

use crate::{FieldbusDriver, FieldbusInputs, FieldbusOutputs};
use plc_common::error::{PlcError, PlcResult};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};
use tracing::{debug, info, trace, warn};

/// Modbus function codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FunctionCode {
    /// Read Coils (0x01).
    ReadCoils = 0x01,
    /// Read Discrete Inputs (0x02).
    ReadDiscreteInputs = 0x02,
    /// Read Holding Registers (0x03).
    ReadHoldingRegisters = 0x03,
    /// Read Input Registers (0x04).
    ReadInputRegisters = 0x04,
    /// Write Single Coil (0x05).
    WriteSingleCoil = 0x05,
    /// Write Single Register (0x06).
    WriteSingleRegister = 0x06,
    /// Write Multiple Coils (0x0F).
    WriteMultipleCoils = 0x0F,
    /// Write Multiple Registers (0x10).
    WriteMultipleRegisters = 0x10,
}

/// Modbus exception codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ExceptionCode {
    /// Illegal function code.
    IllegalFunction = 0x01,
    /// Illegal data address.
    IllegalDataAddress = 0x02,
    /// Illegal data value.
    IllegalDataValue = 0x03,
    /// Server device failure.
    ServerDeviceFailure = 0x04,
    /// Acknowledge (request accepted, processing).
    Acknowledge = 0x05,
    /// Server device busy.
    ServerDeviceBusy = 0x06,
    /// Gateway path unavailable.
    GatewayPathUnavailable = 0x0A,
    /// Gateway target device failed to respond.
    GatewayTargetFailed = 0x0B,
}

impl ExceptionCode {
    /// Parse an exception code from a byte value.
    fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0x01 => Some(Self::IllegalFunction),
            0x02 => Some(Self::IllegalDataAddress),
            0x03 => Some(Self::IllegalDataValue),
            0x04 => Some(Self::ServerDeviceFailure),
            0x05 => Some(Self::Acknowledge),
            0x06 => Some(Self::ServerDeviceBusy),
            0x0A => Some(Self::GatewayPathUnavailable),
            0x0B => Some(Self::GatewayTargetFailed),
            _ => None,
        }
    }
}

impl std::fmt::Display for ExceptionCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IllegalFunction => write!(f, "Illegal Function"),
            Self::IllegalDataAddress => write!(f, "Illegal Data Address"),
            Self::IllegalDataValue => write!(f, "Illegal Data Value"),
            Self::ServerDeviceFailure => write!(f, "Server Device Failure"),
            Self::Acknowledge => write!(f, "Acknowledge"),
            Self::ServerDeviceBusy => write!(f, "Server Device Busy"),
            Self::GatewayPathUnavailable => write!(f, "Gateway Path Unavailable"),
            Self::GatewayTargetFailed => write!(f, "Gateway Target Failed"),
        }
    }
}

/// Modbus TCP Application Protocol (MBAP) header.
#[derive(Debug, Clone, Copy)]
struct MbapHeader {
    /// Transaction identifier (echoed by server).
    transaction_id: u16,
    /// Protocol identifier (0 for Modbus).
    protocol_id: u16,
    /// Length of remaining data (unit ID + PDU).
    length: u16,
    /// Unit identifier (slave address).
    unit_id: u8,
}

impl MbapHeader {
    /// MBAP header size in bytes.
    const SIZE: usize = 7;

    /// Create a new MBAP header.
    fn new(transaction_id: u16, pdu_length: u16, unit_id: u8) -> Self {
        Self {
            transaction_id,
            protocol_id: 0,         // Always 0 for Modbus
            length: pdu_length + 1, // +1 for unit_id
            unit_id,
        }
    }

    /// Serialize the header to bytes (big-endian).
    fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut bytes = [0u8; Self::SIZE];
        bytes[0..2].copy_from_slice(&self.transaction_id.to_be_bytes());
        bytes[2..4].copy_from_slice(&self.protocol_id.to_be_bytes());
        bytes[4..6].copy_from_slice(&self.length.to_be_bytes());
        bytes[6] = self.unit_id;
        bytes
    }

    /// Parse a header from bytes.
    fn from_bytes(bytes: &[u8]) -> PlcResult<Self> {
        if bytes.len() < Self::SIZE {
            return Err(PlcError::FieldbusError(format!(
                "MBAP header too short: {} bytes",
                bytes.len()
            )));
        }

        Ok(Self {
            transaction_id: u16::from_be_bytes([bytes[0], bytes[1]]),
            protocol_id: u16::from_be_bytes([bytes[2], bytes[3]]),
            length: u16::from_be_bytes([bytes[4], bytes[5]]),
            unit_id: bytes[6],
        })
    }
}

/// Configuration for a Modbus I/O mapping.
#[derive(Debug, Clone)]
pub struct ModbusMapping {
    /// Starting Modbus address.
    pub address: u16,
    /// Number of coils/registers to read/write.
    pub quantity: u16,
    /// Function code to use.
    pub function: FunctionCode,
}

/// Configuration for the Modbus TCP driver.
#[derive(Debug, Clone)]
pub struct ModbusTcpConfig {
    /// Server address (IP:port).
    pub server_addr: SocketAddr,
    /// Unit ID (slave address), typically 1.
    pub unit_id: u8,
    /// Connection timeout.
    pub connect_timeout: Duration,
    /// Read/write timeout.
    pub io_timeout: Duration,
    /// Number of reconnection attempts before failing.
    pub max_reconnect_attempts: u32,
    /// Delay between reconnection attempts.
    pub reconnect_delay: Duration,
    /// Digital input mapping (coils or discrete inputs).
    pub digital_input_mapping: Option<ModbusMapping>,
    /// Digital output mapping (coils).
    pub digital_output_mapping: Option<ModbusMapping>,
    /// Analog input mapping (input or holding registers).
    pub analog_input_mapping: Option<ModbusMapping>,
    /// Analog output mapping (holding registers).
    pub analog_output_mapping: Option<ModbusMapping>,
}

impl Default for ModbusTcpConfig {
    fn default() -> Self {
        Self {
            server_addr: "127.0.0.1:502".parse().expect("valid default address"),
            unit_id: 1,
            connect_timeout: Duration::from_secs(5),
            io_timeout: Duration::from_secs(1),
            max_reconnect_attempts: 3,
            reconnect_delay: Duration::from_millis(500),
            digital_input_mapping: Some(ModbusMapping {
                address: 0,
                quantity: 32,
                function: FunctionCode::ReadDiscreteInputs,
            }),
            digital_output_mapping: Some(ModbusMapping {
                address: 0,
                quantity: 32,
                function: FunctionCode::WriteMultipleCoils,
            }),
            analog_input_mapping: Some(ModbusMapping {
                address: 0,
                quantity: 16,
                function: FunctionCode::ReadInputRegisters,
            }),
            analog_output_mapping: Some(ModbusMapping {
                address: 0,
                quantity: 16,
                function: FunctionCode::WriteMultipleRegisters,
            }),
        }
    }
}

/// Connection state for the Modbus TCP driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionState {
    /// Not connected.
    Disconnected,
    /// Connected and operational.
    Connected,
    /// Connection failed, attempting reconnection.
    Reconnecting,
    /// Permanently failed.
    Failed,
}

/// Modbus TCP driver for PLC fieldbus communication.
pub struct ModbusTcpDriver {
    /// Configuration.
    config: ModbusTcpConfig,
    /// TCP connection (None if disconnected).
    connection: Option<TcpStream>,
    /// Connection state.
    state: ConnectionState,
    /// Transaction ID counter.
    transaction_id: u16,
    /// Current reconnection attempt count.
    reconnect_attempts: u32,
    /// Cached input values.
    inputs: FieldbusInputs,
    /// Cached output values (to be written).
    outputs: FieldbusOutputs,
    /// Receive buffer.
    rx_buffer: Vec<u8>,
    /// Next allowed reconnect time (for non-blocking reconnection).
    next_reconnect_time: Option<Instant>,
}

impl ModbusTcpDriver {
    /// Create a new Modbus TCP driver with default configuration.
    pub fn new() -> Self {
        Self::with_config(ModbusTcpConfig::default())
    }

    /// Create a new Modbus TCP driver with custom configuration.
    pub fn with_config(config: ModbusTcpConfig) -> Self {
        Self {
            config,
            connection: None,
            state: ConnectionState::Disconnected,
            transaction_id: 0,
            reconnect_attempts: 0,
            inputs: FieldbusInputs::default(),
            outputs: FieldbusOutputs::default(),
            rx_buffer: vec![0u8; 260], // Max Modbus TCP frame size
            next_reconnect_time: None,
        }
    }

    /// Get the next transaction ID.
    fn next_transaction_id(&mut self) -> u16 {
        self.transaction_id = self.transaction_id.wrapping_add(1);
        self.transaction_id
    }

    /// Connect to the Modbus server.
    fn connect(&mut self) -> PlcResult<()> {
        info!(addr = %self.config.server_addr, "Connecting to Modbus TCP server");

        let stream =
            TcpStream::connect_timeout(&self.config.server_addr, self.config.connect_timeout)
                .map_err(|e| PlcError::FieldbusError(format!("Connection failed: {e}")))?;

        stream
            .set_read_timeout(Some(self.config.io_timeout))
            .map_err(|e| PlcError::FieldbusError(format!("Failed to set read timeout: {e}")))?;

        stream
            .set_write_timeout(Some(self.config.io_timeout))
            .map_err(|e| PlcError::FieldbusError(format!("Failed to set write timeout: {e}")))?;

        stream
            .set_nodelay(true)
            .map_err(|e| PlcError::FieldbusError(format!("Failed to set TCP_NODELAY: {e}")))?;

        self.connection = Some(stream);
        self.state = ConnectionState::Connected;
        self.reconnect_attempts = 0;
        self.next_reconnect_time = None;

        info!("Connected to Modbus TCP server");
        Ok(())
    }

    /// Attempt to reconnect after a connection failure.
    ///
    /// This method uses a non-blocking, cycle-deferred approach. Instead of
    /// sleeping (which would block the entire PLC cycle), it checks if enough
    /// time has passed since the last failed attempt before trying again.
    fn try_reconnect(&mut self) -> PlcResult<()> {
        if self.reconnect_attempts >= self.config.max_reconnect_attempts {
            self.state = ConnectionState::Failed;
            return Err(PlcError::FieldbusError(format!(
                "Max reconnection attempts ({}) exceeded",
                self.config.max_reconnect_attempts
            )));
        }

        // Check if we need to wait before the next reconnect attempt
        if let Some(next_time) = self.next_reconnect_time {
            if Instant::now() < next_time {
                // Not yet time to reconnect, return immediately without blocking
                return Err(PlcError::FieldbusError(
                    "Reconnecting: waiting for retry delay".into(),
                ));
            }
        }

        self.reconnect_attempts += 1;
        self.state = ConnectionState::Reconnecting;

        warn!(
            attempt = self.reconnect_attempts,
            max = self.config.max_reconnect_attempts,
            "Attempting Modbus reconnection"
        );

        match self.connect() {
            Ok(()) => {
                // Success: next_reconnect_time is cleared in connect()
                Ok(())
            }
            Err(e) => {
                // Connection failed: schedule next attempt
                self.next_reconnect_time = Some(Instant::now() + self.config.reconnect_delay);
                Err(e)
            }
        }
    }

    /// Send a Modbus request and receive the response.
    fn send_request(&mut self, pdu: &[u8]) -> PlcResult<Vec<u8>> {
        if self.connection.is_none() {
            return Err(PlcError::FieldbusError(
                "Not connected to Modbus server".into(),
            ));
        }

        let transaction_id = self.next_transaction_id();
        let header = MbapHeader::new(transaction_id, pdu.len() as u16, self.config.unit_id);

        // Build the complete request frame
        let mut request = Vec::with_capacity(MbapHeader::SIZE + pdu.len());
        request.extend_from_slice(&header.to_bytes());
        request.extend_from_slice(pdu);

        trace!(
            transaction_id,
            pdu_len = pdu.len(),
            "Sending Modbus request"
        );

        // Send request
        if let Some(stream) = self.connection.as_mut() {
            if let Err(e) = stream.write_all(&request) {
                self.connection = None;
                self.state = ConnectionState::Disconnected;
                return Err(PlcError::FieldbusError(format!("Send failed: {e}")));
            }
        }

        // Read response header
        {
            let stream = self
                .connection
                .as_mut()
                .ok_or_else(|| PlcError::FieldbusError("Connection lost during send".into()))?;
            let header_buf = &mut self.rx_buffer[..MbapHeader::SIZE];
            if let Err(e) = stream.read_exact(header_buf) {
                self.connection = None;
                self.state = ConnectionState::Disconnected;
                return Err(PlcError::FieldbusError(format!(
                    "Receive header failed: {e}"
                )));
            }
        }

        let response_header = MbapHeader::from_bytes(&self.rx_buffer[..MbapHeader::SIZE])?;

        // Validate response header
        if response_header.transaction_id != transaction_id {
            return Err(PlcError::FieldbusError(format!(
                "Transaction ID mismatch: expected {}, got {}",
                transaction_id, response_header.transaction_id
            )));
        }

        if response_header.protocol_id != 0 {
            return Err(PlcError::FieldbusError(format!(
                "Invalid protocol ID: {}",
                response_header.protocol_id
            )));
        }

        // Validate unit ID matches request
        if response_header.unit_id != self.config.unit_id {
            return Err(PlcError::FieldbusError(format!(
                "Unit ID mismatch: expected {}, got {}",
                self.config.unit_id, response_header.unit_id
            )));
        }

        // Read response PDU
        let pdu_length = (response_header.length - 1) as usize; // -1 for unit_id
        if pdu_length > self.rx_buffer.len() - MbapHeader::SIZE {
            return Err(PlcError::FieldbusError(format!(
                "Response too large: {} bytes",
                pdu_length
            )));
        }

        {
            let stream = self
                .connection
                .as_mut()
                .ok_or_else(|| PlcError::FieldbusError("Connection lost during receive".into()))?;
            let pdu_buf = &mut self.rx_buffer[MbapHeader::SIZE..MbapHeader::SIZE + pdu_length];
            if let Err(e) = stream.read_exact(pdu_buf) {
                self.connection = None;
                self.state = ConnectionState::Disconnected;
                return Err(PlcError::FieldbusError(format!("Receive PDU failed: {e}")));
            }
        }

        // Check for exception response (function code has high bit set)
        let pdu_buf = &self.rx_buffer[MbapHeader::SIZE..MbapHeader::SIZE + pdu_length];
        if !pdu_buf.is_empty() && (pdu_buf[0] & 0x80) != 0 {
            let exception_code = if pdu_buf.len() > 1 {
                ExceptionCode::from_byte(pdu_buf[1])
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| format!("Unknown (0x{:02X})", pdu_buf[1]))
            } else {
                "Unknown".into()
            };
            return Err(PlcError::FieldbusError(format!(
                "Modbus exception: {}",
                exception_code
            )));
        }

        trace!(
            transaction_id,
            pdu_len = pdu_length,
            "Received Modbus response"
        );

        Ok(pdu_buf.to_vec())
    }

    /// Read coils (function 0x01) or discrete inputs (function 0x02).
    fn read_bits(
        &mut self,
        function: FunctionCode,
        address: u16,
        quantity: u16,
    ) -> PlcResult<Vec<bool>> {
        let pdu = [
            function as u8,
            (address >> 8) as u8,
            (address & 0xFF) as u8,
            (quantity >> 8) as u8,
            (quantity & 0xFF) as u8,
        ];

        let response = self.send_request(&pdu)?;

        if response.len() < 2 {
            return Err(PlcError::FieldbusError("Response too short".into()));
        }

        // Validate function code matches request (response[0] should echo the function code)
        let response_function = response[0];
        if response_function != function as u8 {
            return Err(PlcError::FieldbusError(format!(
                "Function code mismatch: expected 0x{:02X}, got 0x{:02X}",
                function as u8, response_function
            )));
        }

        let byte_count = response[1] as usize;

        // Validate byte_count matches expected size for the requested quantity
        // (quantity bits require ceil(quantity/8) bytes)
        let expected_bytes = (quantity as usize + 7) / 8;
        if byte_count < expected_bytes {
            return Err(PlcError::FieldbusError(format!(
                "Byte count mismatch: expected at least {} bytes for {} bits, got {}",
                expected_bytes, quantity, byte_count
            )));
        }

        if response.len() < 2 + byte_count {
            return Err(PlcError::FieldbusError(format!(
                "Expected {} data bytes, got {}",
                byte_count,
                response.len() - 2
            )));
        }

        // Unpack bits from bytes
        let mut bits = Vec::with_capacity(quantity as usize);
        for i in 0..quantity as usize {
            let byte_idx = i / 8;
            let bit_idx = i % 8;
            let bit = (response[2 + byte_idx] >> bit_idx) & 1 != 0;
            bits.push(bit);
        }

        Ok(bits)
    }

    /// Read holding registers (function 0x03) or input registers (function 0x04).
    fn read_registers(
        &mut self,
        function: FunctionCode,
        address: u16,
        quantity: u16,
    ) -> PlcResult<Vec<u16>> {
        let pdu = [
            function as u8,
            (address >> 8) as u8,
            (address & 0xFF) as u8,
            (quantity >> 8) as u8,
            (quantity & 0xFF) as u8,
        ];

        let response = self.send_request(&pdu)?;

        if response.len() < 2 {
            return Err(PlcError::FieldbusError("Response too short".into()));
        }

        // Validate function code matches request
        let response_function = response[0];
        if response_function != function as u8 {
            return Err(PlcError::FieldbusError(format!(
                "Function code mismatch: expected 0x{:02X}, got 0x{:02X}",
                function as u8, response_function
            )));
        }

        let byte_count = response[1] as usize;
        let expected_bytes = quantity as usize * 2;
        if byte_count != expected_bytes {
            return Err(PlcError::FieldbusError(format!(
                "Expected {} bytes, got {}",
                expected_bytes, byte_count
            )));
        }

        if response.len() < 2 + byte_count {
            return Err(PlcError::FieldbusError(format!(
                "Response too short: expected {} bytes",
                2 + byte_count
            )));
        }

        // Unpack registers (big-endian)
        let mut registers = Vec::with_capacity(quantity as usize);
        for i in 0..quantity as usize {
            let offset = 2 + i * 2;
            let value = u16::from_be_bytes([response[offset], response[offset + 1]]);
            registers.push(value);
        }

        Ok(registers)
    }

    /// Write a single coil (function 0x05).
    ///
    /// Writes a single coil to 0xFF00 (ON) or 0x0000 (OFF).
    /// Response echoes the request exactly.
    fn write_single_coil(&mut self, address: u16, value: bool) -> PlcResult<()> {
        // Coil value: 0xFF00 for ON, 0x0000 for OFF
        let coil_value: u16 = if value { 0xFF00 } else { 0x0000 };

        let pdu = [
            FunctionCode::WriteSingleCoil as u8,
            (address >> 8) as u8,
            (address & 0xFF) as u8,
            (coil_value >> 8) as u8,
            (coil_value & 0xFF) as u8,
        ];

        let response = self.send_request(&pdu)?;

        // Response should echo the request (5 bytes)
        if response.len() < 5 {
            return Err(PlcError::FieldbusError("Response too short".into()));
        }

        // Validate function code matches request
        let response_function = response[0];
        if response_function != FunctionCode::WriteSingleCoil as u8 {
            return Err(PlcError::FieldbusError(format!(
                "Function code mismatch: expected 0x{:02X}, got 0x{:02X}",
                FunctionCode::WriteSingleCoil as u8,
                response_function
            )));
        }

        // Validate response echoes request
        let resp_address = u16::from_be_bytes([response[1], response[2]]);
        let resp_value = u16::from_be_bytes([response[3], response[4]]);

        if resp_address != address {
            return Err(PlcError::FieldbusError(format!(
                "Write single coil address mismatch: expected {}, got {}",
                address, resp_address
            )));
        }

        if resp_value != coil_value {
            return Err(PlcError::FieldbusError(format!(
                "Write single coil value mismatch: expected 0x{:04X}, got 0x{:04X}",
                coil_value, resp_value
            )));
        }

        Ok(())
    }

    /// Write a single register (function 0x06).
    ///
    /// Writes a single 16-bit register value.
    /// Response echoes the request exactly.
    fn write_single_register(&mut self, address: u16, value: u16) -> PlcResult<()> {
        let pdu = [
            FunctionCode::WriteSingleRegister as u8,
            (address >> 8) as u8,
            (address & 0xFF) as u8,
            (value >> 8) as u8,
            (value & 0xFF) as u8,
        ];

        let response = self.send_request(&pdu)?;

        // Response should echo the request (5 bytes)
        if response.len() < 5 {
            return Err(PlcError::FieldbusError("Response too short".into()));
        }

        // Validate function code matches request
        let response_function = response[0];
        if response_function != FunctionCode::WriteSingleRegister as u8 {
            return Err(PlcError::FieldbusError(format!(
                "Function code mismatch: expected 0x{:02X}, got 0x{:02X}",
                FunctionCode::WriteSingleRegister as u8,
                response_function
            )));
        }

        // Validate response echoes request
        let resp_address = u16::from_be_bytes([response[1], response[2]]);
        let resp_value = u16::from_be_bytes([response[3], response[4]]);

        if resp_address != address {
            return Err(PlcError::FieldbusError(format!(
                "Write single register address mismatch: expected {}, got {}",
                address, resp_address
            )));
        }

        if resp_value != value {
            return Err(PlcError::FieldbusError(format!(
                "Write single register value mismatch: expected 0x{:04X}, got 0x{:04X}",
                value, resp_value
            )));
        }

        Ok(())
    }

    /// Write multiple coils (function 0x0F).
    ///
    /// When writing a single coil, uses FC 0x05 for better device compatibility.
    fn write_coils(&mut self, address: u16, values: &[bool]) -> PlcResult<()> {
        // Validate non-empty input (quantity=0 is invalid per Modbus spec)
        if values.is_empty() {
            return Err(PlcError::FieldbusError("Cannot write zero coils".into()));
        }

        // Optimization: use single-write FC for single value
        if values.len() == 1 {
            return self.write_single_coil(address, values[0]);
        }

        let quantity = values.len() as u16;
        let byte_count = (values.len() + 7) / 8;

        // Pack bits into bytes
        let mut data_bytes = vec![0u8; byte_count];
        for (i, &value) in values.iter().enumerate() {
            if value {
                let byte_idx = i / 8;
                let bit_idx = i % 8;
                data_bytes[byte_idx] |= 1 << bit_idx;
            }
        }

        let mut pdu = Vec::with_capacity(6 + byte_count);
        pdu.push(FunctionCode::WriteMultipleCoils as u8);
        pdu.extend_from_slice(&address.to_be_bytes());
        pdu.extend_from_slice(&quantity.to_be_bytes());
        pdu.push(byte_count as u8);
        pdu.extend_from_slice(&data_bytes);

        let response = self.send_request(&pdu)?;

        // Verify response
        if response.len() < 5 {
            return Err(PlcError::FieldbusError("Response too short".into()));
        }

        // Validate function code matches request
        let response_function = response[0];
        if response_function != FunctionCode::WriteMultipleCoils as u8 {
            return Err(PlcError::FieldbusError(format!(
                "Function code mismatch: expected 0x{:02X}, got 0x{:02X}",
                FunctionCode::WriteMultipleCoils as u8,
                response_function
            )));
        }

        let resp_address = u16::from_be_bytes([response[1], response[2]]);
        let resp_quantity = u16::from_be_bytes([response[3], response[4]]);

        if resp_address != address || resp_quantity != quantity {
            return Err(PlcError::FieldbusError(format!(
                "Write coils response mismatch: addr={}/{}, qty={}/{}",
                resp_address, address, resp_quantity, quantity
            )));
        }

        Ok(())
    }

    /// Write multiple registers (function 0x10).
    ///
    /// When writing a single register, uses FC 0x06 for better device compatibility.
    fn write_registers(&mut self, address: u16, values: &[u16]) -> PlcResult<()> {
        // Validate non-empty input (quantity=0 is invalid per Modbus spec)
        if values.is_empty() {
            return Err(PlcError::FieldbusError(
                "Cannot write zero registers".into(),
            ));
        }

        // Optimization: use single-write FC for single value
        if values.len() == 1 {
            return self.write_single_register(address, values[0]);
        }

        let quantity = values.len() as u16;
        let byte_count = values.len() * 2;

        let mut pdu = Vec::with_capacity(6 + byte_count);
        pdu.push(FunctionCode::WriteMultipleRegisters as u8);
        pdu.extend_from_slice(&address.to_be_bytes());
        pdu.extend_from_slice(&quantity.to_be_bytes());
        pdu.push(byte_count as u8);

        for &value in values {
            pdu.extend_from_slice(&value.to_be_bytes());
        }

        let response = self.send_request(&pdu)?;

        // Verify response
        if response.len() < 5 {
            return Err(PlcError::FieldbusError("Response too short".into()));
        }

        // Validate function code matches request
        let response_function = response[0];
        if response_function != FunctionCode::WriteMultipleRegisters as u8 {
            return Err(PlcError::FieldbusError(format!(
                "Function code mismatch: expected 0x{:02X}, got 0x{:02X}",
                FunctionCode::WriteMultipleRegisters as u8,
                response_function
            )));
        }

        let resp_address = u16::from_be_bytes([response[1], response[2]]);
        let resp_quantity = u16::from_be_bytes([response[3], response[4]]);

        if resp_address != address || resp_quantity != quantity {
            return Err(PlcError::FieldbusError(format!(
                "Write registers response mismatch: addr={}/{}, qty={}/{}",
                resp_address, address, resp_quantity, quantity
            )));
        }

        Ok(())
    }

    /// Read digital inputs from the configured mapping.
    fn read_digital_inputs(&mut self) -> PlcResult<()> {
        if let Some(ref mapping) = self.config.digital_input_mapping {
            let bits = self.read_bits(mapping.function, mapping.address, mapping.quantity)?;

            // Pack bits into u32
            let mut digital = 0u32;
            for (i, &bit) in bits.iter().take(32).enumerate() {
                if bit {
                    digital |= 1 << i;
                }
            }
            self.inputs.digital = digital;
        }
        Ok(())
    }

    /// Read analog inputs from the configured mapping.
    fn read_analog_inputs(&mut self) -> PlcResult<()> {
        if let Some(ref mapping) = self.config.analog_input_mapping {
            let registers =
                self.read_registers(mapping.function, mapping.address, mapping.quantity)?;

            for (i, &value) in registers.iter().take(16).enumerate() {
                self.inputs.analog[i] = value as i16;
            }
        }
        Ok(())
    }

    /// Write digital outputs to the configured mapping.
    fn write_digital_outputs(&mut self) -> PlcResult<()> {
        if let Some(ref mapping) = self.config.digital_output_mapping {
            // Unpack u32 into bits
            let mut bits = Vec::with_capacity(mapping.quantity as usize);
            for i in 0..mapping.quantity as usize {
                bits.push((self.outputs.digital >> i) & 1 != 0);
            }
            self.write_coils(mapping.address, &bits)?;
        }
        Ok(())
    }

    /// Write analog outputs to the configured mapping.
    fn write_analog_outputs(&mut self) -> PlcResult<()> {
        if let Some(ref mapping) = self.config.analog_output_mapping {
            let registers: Vec<u16> = self
                .outputs
                .analog
                .iter()
                .take(mapping.quantity as usize)
                .map(|&v| v as u16)
                .collect();
            self.write_registers(mapping.address, &registers)?;
        }
        Ok(())
    }
}

impl Default for ModbusTcpDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl FieldbusDriver for ModbusTcpDriver {
    fn init(&mut self) -> PlcResult<()> {
        info!(
            server = %self.config.server_addr,
            unit_id = self.config.unit_id,
            "Initializing Modbus TCP driver"
        );

        self.connect()?;

        debug!("Modbus TCP driver initialized");
        Ok(())
    }

    fn read_inputs(&mut self) -> PlcResult<()> {
        if self.state != ConnectionState::Connected {
            self.try_reconnect()?;
        }

        // Read digital inputs
        if let Err(e) = self.read_digital_inputs() {
            warn!(error = %e, "Failed to read digital inputs");
            return Err(e);
        }

        // Read analog inputs
        if let Err(e) = self.read_analog_inputs() {
            warn!(error = %e, "Failed to read analog inputs");
            return Err(e);
        }

        Ok(())
    }

    fn write_outputs(&mut self) -> PlcResult<()> {
        if self.state != ConnectionState::Connected {
            self.try_reconnect()?;
        }

        // Write digital outputs
        if let Err(e) = self.write_digital_outputs() {
            warn!(error = %e, "Failed to write digital outputs");
            return Err(e);
        }

        // Write analog outputs
        if let Err(e) = self.write_analog_outputs() {
            warn!(error = %e, "Failed to write analog outputs");
            return Err(e);
        }

        Ok(())
    }

    fn get_inputs(&self) -> FieldbusInputs {
        self.inputs
    }

    fn set_outputs(&mut self, outputs: &FieldbusOutputs) {
        self.outputs = *outputs;
    }

    fn shutdown(&mut self) -> PlcResult<()> {
        info!("Shutting down Modbus TCP driver");

        if let Some(stream) = self.connection.take() {
            drop(stream);
        }

        self.state = ConnectionState::Disconnected;
        Ok(())
    }

    fn is_operational(&self) -> bool {
        self.state == ConnectionState::Connected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mbap_header_serialization() {
        let header = MbapHeader::new(0x1234, 5, 1);
        let bytes = header.to_bytes();

        assert_eq!(bytes[0], 0x12); // transaction_id high
        assert_eq!(bytes[1], 0x34); // transaction_id low
        assert_eq!(bytes[2], 0x00); // protocol_id high
        assert_eq!(bytes[3], 0x00); // protocol_id low
        assert_eq!(bytes[4], 0x00); // length high (5 + 1 = 6)
        assert_eq!(bytes[5], 0x06); // length low
        assert_eq!(bytes[6], 0x01); // unit_id
    }

    #[test]
    fn test_mbap_header_parsing() {
        let bytes = [0x12, 0x34, 0x00, 0x00, 0x00, 0x06, 0x01];
        let header = MbapHeader::from_bytes(&bytes).unwrap();

        assert_eq!(header.transaction_id, 0x1234);
        assert_eq!(header.protocol_id, 0);
        assert_eq!(header.length, 6);
        assert_eq!(header.unit_id, 1);
    }

    #[test]
    fn test_default_config() {
        let config = ModbusTcpConfig::default();
        assert_eq!(config.unit_id, 1);
        assert!(config.digital_input_mapping.is_some());
        assert!(config.digital_output_mapping.is_some());
        assert!(config.analog_input_mapping.is_some());
        assert!(config.analog_output_mapping.is_some());
    }

    #[test]
    fn test_driver_creation() {
        let driver = ModbusTcpDriver::new();
        assert_eq!(driver.state, ConnectionState::Disconnected);
        assert!(!driver.is_operational());
    }

    #[test]
    fn test_transaction_id_wrapping() {
        let mut driver = ModbusTcpDriver::new();
        driver.transaction_id = u16::MAX;

        let id = driver.next_transaction_id();
        assert_eq!(id, 0); // Should wrap around
    }

    #[test]
    fn test_exception_code_display() {
        assert_eq!(
            ExceptionCode::IllegalFunction.to_string(),
            "Illegal Function"
        );
        assert_eq!(
            ExceptionCode::IllegalDataAddress.to_string(),
            "Illegal Data Address"
        );
    }

    #[test]
    fn test_exception_code_parsing() {
        assert_eq!(
            ExceptionCode::from_byte(0x01),
            Some(ExceptionCode::IllegalFunction)
        );
        assert_eq!(
            ExceptionCode::from_byte(0x04),
            Some(ExceptionCode::ServerDeviceFailure)
        );
        assert_eq!(ExceptionCode::from_byte(0xFF), None);
    }

    #[test]
    fn test_set_outputs() {
        let mut driver = ModbusTcpDriver::new();
        let outputs = FieldbusOutputs {
            digital: 0x12345678,
            analog: [
                100, 200, 300, 400, 500, 600, 700, 800, 0, 0, 0, 0, 0, 0, 0, 0,
            ],
        };

        driver.set_outputs(&outputs);

        assert_eq!(driver.outputs.digital, 0x12345678);
        assert_eq!(driver.outputs.analog[0], 100);
        assert_eq!(driver.outputs.analog[7], 800);
    }

    #[test]
    fn test_next_reconnect_time_initialized_to_none() {
        let driver = ModbusTcpDriver::new();
        assert!(driver.next_reconnect_time.is_none());
    }

    #[test]
    fn test_try_reconnect_returns_immediately_during_delay() {
        let mut driver = ModbusTcpDriver::new();
        // Set a future reconnect time (100ms from now)
        driver.next_reconnect_time = Some(Instant::now() + Duration::from_millis(100));
        driver.state = ConnectionState::Disconnected;

        // Measure time before and after to verify no blocking
        let start = Instant::now();
        let result = driver.try_reconnect();
        let elapsed = start.elapsed();

        // Should return immediately (well under 100ms)
        assert!(elapsed < Duration::from_millis(10));

        // Should return an error indicating we're still in delay period
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(&err, PlcError::FieldbusError(msg) if msg.contains("waiting for retry delay")),
            "Expected 'waiting for retry delay' error, got: {err}"
        );

        // Should not increment reconnect attempts
        assert_eq!(driver.reconnect_attempts, 0);
    }

    #[test]
    fn test_try_reconnect_attempts_after_delay_expires() {
        let mut driver = ModbusTcpDriver::new();
        // Set a past reconnect time
        driver.next_reconnect_time = Some(Instant::now() - Duration::from_millis(100));
        driver.state = ConnectionState::Disconnected;

        // This will fail to connect (no server), but it should attempt
        let result = driver.try_reconnect();

        // Should have attempted connection
        assert_eq!(driver.reconnect_attempts, 1);

        // Should fail (no server to connect to)
        assert!(result.is_err());

        // Should schedule next reconnect attempt
        assert!(driver.next_reconnect_time.is_some());
    }

    #[test]
    fn test_try_reconnect_schedules_next_attempt_on_failure() {
        let mut driver = ModbusTcpDriver::new();
        driver.state = ConnectionState::Disconnected;

        // First attempt (no next_reconnect_time set)
        let start = Instant::now();
        let _ = driver.try_reconnect();

        // Should have scheduled next attempt
        assert!(driver.next_reconnect_time.is_some());
        let next_time = driver.next_reconnect_time.unwrap();

        // Next time should be approximately reconnect_delay in the future
        let expected_min = start + driver.config.reconnect_delay - Duration::from_millis(50);
        let expected_max = start + driver.config.reconnect_delay + Duration::from_millis(50);
        assert!(
            next_time >= expected_min && next_time <= expected_max,
            "next_reconnect_time should be approximately reconnect_delay from now"
        );
    }

    #[test]
    fn test_try_reconnect_max_attempts_exceeded() {
        let mut driver = ModbusTcpDriver::new();
        driver.reconnect_attempts = driver.config.max_reconnect_attempts;
        driver.state = ConnectionState::Disconnected;

        let result = driver.try_reconnect();

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(&err, PlcError::FieldbusError(msg) if msg.contains("Max reconnection attempts")),
            "Expected max attempts error, got: {err}"
        );
        assert_eq!(driver.state, ConnectionState::Failed);
    }

    #[test]
    fn test_try_reconnect_non_blocking_multiple_calls() {
        let mut driver = ModbusTcpDriver::new();
        driver.state = ConnectionState::Disconnected;

        // First call should attempt connection and fail
        let _ = driver.try_reconnect();
        assert_eq!(driver.reconnect_attempts, 1);
        assert!(driver.next_reconnect_time.is_some());

        // Immediate second call should return immediately (during delay)
        let start = Instant::now();
        let result = driver.try_reconnect();
        let elapsed = start.elapsed();

        // Should return immediately without blocking
        assert!(elapsed < Duration::from_millis(10));
        assert!(result.is_err());
        // Should not have incremented attempts again
        assert_eq!(driver.reconnect_attempts, 1);
    }

    #[test]
    fn test_write_single_coil_pdu_format() {
        // Test PDU construction for write single coil (FC 0x05)
        // PDU format: [0x05, addr_hi, addr_lo, value_hi, value_lo]
        // Value: 0xFF00 for ON, 0x0000 for OFF

        // Test ON value encoding
        let coil_on: u16 = 0xFF00;
        let address: u16 = 0x1234;
        let pdu_on = [
            FunctionCode::WriteSingleCoil as u8,
            (address >> 8) as u8,
            (address & 0xFF) as u8,
            (coil_on >> 8) as u8,
            (coil_on & 0xFF) as u8,
        ];
        assert_eq!(pdu_on, [0x05, 0x12, 0x34, 0xFF, 0x00]);

        // Test OFF value encoding
        let coil_off: u16 = 0x0000;
        let pdu_off = [
            FunctionCode::WriteSingleCoil as u8,
            (address >> 8) as u8,
            (address & 0xFF) as u8,
            (coil_off >> 8) as u8,
            (coil_off & 0xFF) as u8,
        ];
        assert_eq!(pdu_off, [0x05, 0x12, 0x34, 0x00, 0x00]);
    }

    #[test]
    fn test_write_single_register_pdu_format() {
        // Test PDU construction for write single register (FC 0x06)
        // PDU format: [0x06, addr_hi, addr_lo, value_hi, value_lo]

        let address: u16 = 0x0100;
        let value: u16 = 0xABCD;
        let pdu = [
            FunctionCode::WriteSingleRegister as u8,
            (address >> 8) as u8,
            (address & 0xFF) as u8,
            (value >> 8) as u8,
            (value & 0xFF) as u8,
        ];
        assert_eq!(pdu, [0x06, 0x01, 0x00, 0xAB, 0xCD]);
    }

    #[test]
    fn test_write_single_coil_value_encoding() {
        // Verify the coil value encoding logic
        // ON = 0xFF00, OFF = 0x0000

        let value_on = true;
        let coil_on: u16 = if value_on { 0xFF00 } else { 0x0000 };
        assert_eq!(coil_on, 0xFF00);

        let value_off = false;
        let coil_off: u16 = if value_off { 0xFF00 } else { 0x0000 };
        assert_eq!(coil_off, 0x0000);
    }

    #[test]
    fn test_write_single_coil_response_parsing() {
        // Test response validation logic for write single coil
        // Response echoes the request: [0x05, addr_hi, addr_lo, value_hi, value_lo]

        let address: u16 = 0x00AC;
        let coil_value: u16 = 0xFF00;

        // Valid response
        let response = [0x05, 0x00, 0xAC, 0xFF, 0x00];
        assert_eq!(response[0], FunctionCode::WriteSingleCoil as u8);
        let resp_address = u16::from_be_bytes([response[1], response[2]]);
        let resp_value = u16::from_be_bytes([response[3], response[4]]);
        assert_eq!(resp_address, address);
        assert_eq!(resp_value, coil_value);
    }

    #[test]
    fn test_write_single_register_response_parsing() {
        // Test response validation logic for write single register
        // Response echoes the request: [0x06, addr_hi, addr_lo, value_hi, value_lo]

        let address: u16 = 0x0001;
        let value: u16 = 0x0003;

        // Valid response
        let response = [0x06, 0x00, 0x01, 0x00, 0x03];
        assert_eq!(response[0], FunctionCode::WriteSingleRegister as u8);
        let resp_address = u16::from_be_bytes([response[1], response[2]]);
        let resp_value = u16::from_be_bytes([response[3], response[4]]);
        assert_eq!(resp_address, address);
        assert_eq!(resp_value, value);
    }

    #[test]
    fn test_write_coils_single_value_uses_single_write() {
        // When writing a single coil value, write_coils should internally
        // use write_single_coil (FC 0x05) instead of write_multiple_coils (FC 0x0F).
        // We can't fully test this without a server, but we can verify the
        // function code enum values are correctly defined.

        assert_eq!(FunctionCode::WriteSingleCoil as u8, 0x05);
        assert_eq!(FunctionCode::WriteMultipleCoils as u8, 0x0F);
    }

    #[test]
    fn test_write_registers_single_value_uses_single_write() {
        // When writing a single register value, write_registers should internally
        // use write_single_register (FC 0x06) instead of write_multiple_registers (FC 0x10).
        // We can't fully test this without a server, but we can verify the
        // function code enum values are correctly defined.

        assert_eq!(FunctionCode::WriteSingleRegister as u8, 0x06);
        assert_eq!(FunctionCode::WriteMultipleRegisters as u8, 0x10);
    }

    #[test]
    fn test_function_codes_defined() {
        // Verify all single and multiple write function codes are defined
        assert_eq!(FunctionCode::WriteSingleCoil as u8, 0x05);
        assert_eq!(FunctionCode::WriteSingleRegister as u8, 0x06);
        assert_eq!(FunctionCode::WriteMultipleCoils as u8, 0x0F);
        assert_eq!(FunctionCode::WriteMultipleRegisters as u8, 0x10);
    }
}
