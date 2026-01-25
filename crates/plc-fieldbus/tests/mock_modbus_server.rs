//! Mock Modbus TCP server for integration testing.
//!
//! Provides a configurable TCP server that speaks the Modbus TCP protocol,
//! allowing integration tests to verify client behavior against a real
//! network connection with controllable fault injection.
//!
//! # Example
//!
//! ```ignore
//! use mock_modbus_server::{MockModbusServer, MockBehavior};
//!
//! let server = MockModbusServer::start(MockBehavior::Normal).unwrap();
//! let addr = server.local_addr();
//!
//! // Connect your Modbus client to `addr` and run tests
//! ```

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Modbus function codes supported by the mock server.
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

impl FunctionCode {
    /// Parse a function code from a byte value.
    fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0x01 => Some(Self::ReadCoils),
            0x02 => Some(Self::ReadDiscreteInputs),
            0x03 => Some(Self::ReadHoldingRegisters),
            0x04 => Some(Self::ReadInputRegisters),
            0x05 => Some(Self::WriteSingleCoil),
            0x06 => Some(Self::WriteSingleRegister),
            0x0F => Some(Self::WriteMultipleCoils),
            0x10 => Some(Self::WriteMultipleRegisters),
            _ => None,
        }
    }
}

/// Configurable behavior for the mock server to enable fault injection testing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockBehavior {
    /// Normal operation - respond correctly to all requests.
    Normal,
    /// Delay response by the specified number of milliseconds.
    DelayMs(u64),
    /// Return a Modbus exception response with the given code.
    Exception(u8),
    /// Accept the connection, receive the request, then drop the connection.
    DropConnection,
    /// Send a malformed/corrupted response.
    CorruptResponse,
    /// Return response with wrong transaction ID.
    WrongTransactionId,
    /// Return response with wrong unit ID.
    WrongUnitId,
}

/// Storage for Modbus register/coil values.
#[derive(Debug, Clone)]
pub struct ModbusStorage {
    /// Coils (read/write bits) - addresses 0-255.
    pub coils: [bool; 256],
    /// Discrete inputs (read-only bits) - addresses 0-255.
    pub discrete_inputs: [bool; 256],
    /// Holding registers (read/write 16-bit) - addresses 0-255.
    pub holding_registers: [u16; 256],
    /// Input registers (read-only 16-bit) - addresses 0-255.
    pub input_registers: [u16; 256],
}

impl Default for ModbusStorage {
    fn default() -> Self {
        Self {
            coils: [false; 256],
            discrete_inputs: [false; 256],
            holding_registers: [0u16; 256],
            input_registers: [0u16; 256],
        }
    }
}

/// Thread-safe storage wrapper.
type SharedStorage = Arc<Mutex<ModbusStorage>>;

/// MBAP (Modbus Application Protocol) header structure.
#[derive(Debug, Clone, Copy)]
struct MbapHeader {
    transaction_id: u16,
    protocol_id: u16,
    length: u16,
    unit_id: u8,
}

impl MbapHeader {
    const SIZE: usize = 7;

    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < Self::SIZE {
            return None;
        }
        Some(Self {
            transaction_id: u16::from_be_bytes([bytes[0], bytes[1]]),
            protocol_id: u16::from_be_bytes([bytes[2], bytes[3]]),
            length: u16::from_be_bytes([bytes[4], bytes[5]]),
            unit_id: bytes[6],
        })
    }

    fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut bytes = [0u8; Self::SIZE];
        bytes[0..2].copy_from_slice(&self.transaction_id.to_be_bytes());
        bytes[2..4].copy_from_slice(&self.protocol_id.to_be_bytes());
        bytes[4..6].copy_from_slice(&self.length.to_be_bytes());
        bytes[6] = self.unit_id;
        bytes
    }
}

/// A mock Modbus TCP server for integration testing.
///
/// The server binds to a localhost port (dynamically allocated) and responds
/// to Modbus TCP requests according to the configured behavior. It maintains
/// stateful storage for coils and registers to enable realistic testing.
pub struct MockModbusServer {
    /// The address the server is listening on.
    local_addr: SocketAddr,
    /// Signal to stop the server thread.
    stop_signal: Arc<AtomicBool>,
    /// Server thread handle.
    thread_handle: Option<JoinHandle<()>>,
    /// Shared storage for register/coil values.
    storage: SharedStorage,
    /// Current behavior configuration.
    behavior: Arc<Mutex<MockBehavior>>,
}

impl MockModbusServer {
    /// Start a new mock Modbus server with the specified behavior.
    ///
    /// The server binds to `127.0.0.1:0` for dynamic port allocation.
    /// Use `local_addr()` to get the actual bound address.
    pub fn start(behavior: MockBehavior) -> std::io::Result<Self> {
        Self::start_with_storage(behavior, ModbusStorage::default())
    }

    /// Start a new mock Modbus server with custom initial storage.
    pub fn start_with_storage(
        behavior: MockBehavior,
        initial_storage: ModbusStorage,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let local_addr = listener.local_addr()?;

        // Set non-blocking so we can check stop signal
        listener.set_nonblocking(true)?;

        let stop_signal = Arc::new(AtomicBool::new(false));
        let storage = Arc::new(Mutex::new(initial_storage));
        let behavior = Arc::new(Mutex::new(behavior));

        let stop_clone = stop_signal.clone();
        let storage_clone = storage.clone();
        let behavior_clone = behavior.clone();

        let thread_handle = thread::spawn(move || {
            Self::server_loop(listener, stop_clone, storage_clone, behavior_clone);
        });

        Ok(Self {
            local_addr,
            stop_signal,
            thread_handle: Some(thread_handle),
            storage,
            behavior,
        })
    }

    /// Get the local address the server is bound to.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Change the server behavior at runtime.
    pub fn set_behavior(&self, behavior: MockBehavior) {
        if let Ok(mut b) = self.behavior.lock() {
            *b = behavior;
        }
    }

    /// Get the current behavior.
    pub fn behavior(&self) -> MockBehavior {
        self.behavior.lock().map(|b| *b).unwrap_or(MockBehavior::Normal)
    }

    /// Access the storage for reading/writing values.
    ///
    /// # Panics
    ///
    /// Panics if the storage mutex is poisoned.
    pub fn with_storage<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut ModbusStorage) -> R,
    {
        let mut storage = self.storage.lock().expect("storage mutex poisoned");
        f(&mut storage)
    }

    /// Set a coil value.
    pub fn set_coil(&self, address: usize, value: bool) {
        self.with_storage(|s| {
            if address < s.coils.len() {
                s.coils[address] = value;
            }
        });
    }

    /// Set a discrete input value.
    pub fn set_discrete_input(&self, address: usize, value: bool) {
        self.with_storage(|s| {
            if address < s.discrete_inputs.len() {
                s.discrete_inputs[address] = value;
            }
        });
    }

    /// Set a holding register value.
    pub fn set_holding_register(&self, address: usize, value: u16) {
        self.with_storage(|s| {
            if address < s.holding_registers.len() {
                s.holding_registers[address] = value;
            }
        });
    }

    /// Set an input register value.
    pub fn set_input_register(&self, address: usize, value: u16) {
        self.with_storage(|s| {
            if address < s.input_registers.len() {
                s.input_registers[address] = value;
            }
        });
    }

    /// Get a coil value.
    pub fn get_coil(&self, address: usize) -> bool {
        self.with_storage(|s| s.coils.get(address).copied().unwrap_or(false))
    }

    /// Get a holding register value.
    pub fn get_holding_register(&self, address: usize) -> u16 {
        self.with_storage(|s| s.holding_registers.get(address).copied().unwrap_or(0))
    }

    /// Stop the server and wait for the thread to finish.
    pub fn stop(mut self) {
        self.stop_signal.store(true, Ordering::SeqCst);
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }

    /// Server main loop.
    fn server_loop(
        listener: TcpListener,
        stop_signal: Arc<AtomicBool>,
        storage: SharedStorage,
        behavior: Arc<Mutex<MockBehavior>>,
    ) {
        while !stop_signal.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, _addr)) => {
                    // Handle each connection in a separate thread for concurrent test support
                    let stop_clone = stop_signal.clone();
                    let storage_clone = storage.clone();
                    let behavior_clone = behavior.clone();

                    thread::spawn(move || {
                        Self::handle_connection(stream, stop_clone, storage_clone, behavior_clone);
                    });
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // No connection ready, sleep briefly and retry
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => {
                    // Other error, continue
                    thread::sleep(Duration::from_millis(10));
                }
            }
        }
    }

    /// Handle a single client connection.
    fn handle_connection(
        mut stream: TcpStream,
        stop_signal: Arc<AtomicBool>,
        storage: SharedStorage,
        behavior: Arc<Mutex<MockBehavior>>,
    ) {
        // Set reasonable timeout for reads
        let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));

        let mut buffer = [0u8; 260]; // Max Modbus TCP frame

        while !stop_signal.load(Ordering::SeqCst) {
            // Read MBAP header first
            match stream.read_exact(&mut buffer[..MbapHeader::SIZE]) {
                Ok(()) => {}
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(_) => {
                    // Connection closed or error
                    return;
                }
            }

            let header = match MbapHeader::from_bytes(&buffer[..MbapHeader::SIZE]) {
                Some(h) => h,
                None => continue,
            };

            // Validate protocol ID (must be 0 for Modbus)
            if header.protocol_id != 0 {
                continue;
            }

            // Read PDU (length includes unit_id, which we already have)
            let pdu_length = header.length.saturating_sub(1) as usize;
            if pdu_length == 0 || pdu_length > 253 {
                continue;
            }

            match stream.read_exact(&mut buffer[MbapHeader::SIZE..MbapHeader::SIZE + pdu_length]) {
                Ok(()) => {}
                Err(_) => return,
            }

            let pdu = &buffer[MbapHeader::SIZE..MbapHeader::SIZE + pdu_length];

            // Get current behavior
            let current_behavior = behavior.lock().map(|b| *b).unwrap_or(MockBehavior::Normal);

            // Handle behavior-based fault injection
            match current_behavior {
                MockBehavior::DelayMs(ms) => {
                    thread::sleep(Duration::from_millis(ms));
                }
                MockBehavior::DropConnection => {
                    // Just return to drop the connection after receiving
                    return;
                }
                MockBehavior::CorruptResponse => {
                    // Send garbage
                    let garbage = [0xFF, 0xFE, 0xFD, 0xFC, 0xFB];
                    let _ = stream.write_all(&garbage);
                    continue;
                }
                MockBehavior::WrongTransactionId => {
                    // Process normally but with wrong transaction ID
                    if let Some(response) =
                        Self::process_request(pdu, &storage, header.unit_id, MockBehavior::Normal)
                    {
                        let resp_header = MbapHeader {
                            transaction_id: header.transaction_id.wrapping_add(1), // Wrong!
                            protocol_id: 0,
                            length: (response.len() + 1) as u16,
                            unit_id: header.unit_id,
                        };
                        let mut frame = Vec::with_capacity(MbapHeader::SIZE + response.len());
                        frame.extend_from_slice(&resp_header.to_bytes());
                        frame.extend_from_slice(&response);
                        let _ = stream.write_all(&frame);
                    }
                    continue;
                }
                MockBehavior::WrongUnitId => {
                    // Process normally but with wrong unit ID
                    if let Some(response) =
                        Self::process_request(pdu, &storage, header.unit_id, MockBehavior::Normal)
                    {
                        let resp_header = MbapHeader {
                            transaction_id: header.transaction_id,
                            protocol_id: 0,
                            length: (response.len() + 1) as u16,
                            unit_id: header.unit_id.wrapping_add(1), // Wrong!
                        };
                        let mut frame = Vec::with_capacity(MbapHeader::SIZE + response.len());
                        frame.extend_from_slice(&resp_header.to_bytes());
                        frame.extend_from_slice(&response);
                        let _ = stream.write_all(&frame);
                    }
                    continue;
                }
                _ => {}
            }

            // Process the request and generate response
            if let Some(response) =
                Self::process_request(pdu, &storage, header.unit_id, current_behavior)
            {
                let resp_header = MbapHeader {
                    transaction_id: header.transaction_id,
                    protocol_id: 0,
                    length: (response.len() + 1) as u16,
                    unit_id: header.unit_id,
                };

                let mut frame = Vec::with_capacity(MbapHeader::SIZE + response.len());
                frame.extend_from_slice(&resp_header.to_bytes());
                frame.extend_from_slice(&response);

                if stream.write_all(&frame).is_err() {
                    return;
                }
            }
        }
    }

    /// Process a Modbus PDU and return the response PDU.
    fn process_request(
        pdu: &[u8],
        storage: &SharedStorage,
        _unit_id: u8,
        behavior: MockBehavior,
    ) -> Option<Vec<u8>> {
        if pdu.is_empty() {
            return None;
        }

        let function_code = pdu[0];

        // Handle exception behavior
        if let MockBehavior::Exception(code) = behavior {
            return Some(vec![function_code | 0x80, code]);
        }

        // Parse and handle based on function code
        match FunctionCode::from_byte(function_code) {
            Some(FunctionCode::ReadCoils) => {
                Self::handle_read_bits(pdu, storage, true)
            }
            Some(FunctionCode::ReadDiscreteInputs) => {
                Self::handle_read_bits(pdu, storage, false)
            }
            Some(FunctionCode::ReadHoldingRegisters) => {
                Self::handle_read_registers(pdu, storage, true)
            }
            Some(FunctionCode::ReadInputRegisters) => {
                Self::handle_read_registers(pdu, storage, false)
            }
            Some(FunctionCode::WriteSingleCoil) => {
                Self::handle_write_single_coil(pdu, storage)
            }
            Some(FunctionCode::WriteSingleRegister) => {
                Self::handle_write_single_register(pdu, storage)
            }
            Some(FunctionCode::WriteMultipleCoils) => {
                Self::handle_write_multiple_coils(pdu, storage)
            }
            Some(FunctionCode::WriteMultipleRegisters) => {
                Self::handle_write_multiple_registers(pdu, storage)
            }
            None => {
                // Unknown function code - return exception
                Some(vec![function_code | 0x80, 0x01]) // Illegal Function
            }
        }
    }

    /// Handle Read Coils (0x01) or Read Discrete Inputs (0x02).
    fn handle_read_bits(pdu: &[u8], storage: &SharedStorage, is_coils: bool) -> Option<Vec<u8>> {
        if pdu.len() < 5 {
            return Some(vec![pdu[0] | 0x80, 0x03]); // Illegal Data Value
        }

        let function_code = pdu[0];
        let start_address = u16::from_be_bytes([pdu[1], pdu[2]]) as usize;
        let quantity = u16::from_be_bytes([pdu[3], pdu[4]]) as usize;

        // Validate quantity (1-2000 for coils)
        if quantity == 0 || quantity > 2000 {
            return Some(vec![function_code | 0x80, 0x03]); // Illegal Data Value
        }

        // Check address range
        if start_address + quantity > 256 {
            return Some(vec![function_code | 0x80, 0x02]); // Illegal Data Address
        }

        let storage = storage.lock().ok()?;
        let bits = if is_coils {
            &storage.coils[start_address..start_address + quantity]
        } else {
            &storage.discrete_inputs[start_address..start_address + quantity]
        };

        // Pack bits into bytes
        let byte_count = (quantity + 7) / 8;
        let mut data = vec![0u8; byte_count];
        for (i, &bit) in bits.iter().enumerate() {
            if bit {
                data[i / 8] |= 1 << (i % 8);
            }
        }

        let mut response = Vec::with_capacity(2 + byte_count);
        response.push(function_code);
        response.push(byte_count as u8);
        response.extend_from_slice(&data);

        Some(response)
    }

    /// Handle Read Holding Registers (0x03) or Read Input Registers (0x04).
    fn handle_read_registers(
        pdu: &[u8],
        storage: &SharedStorage,
        is_holding: bool,
    ) -> Option<Vec<u8>> {
        if pdu.len() < 5 {
            return Some(vec![pdu[0] | 0x80, 0x03]); // Illegal Data Value
        }

        let function_code = pdu[0];
        let start_address = u16::from_be_bytes([pdu[1], pdu[2]]) as usize;
        let quantity = u16::from_be_bytes([pdu[3], pdu[4]]) as usize;

        // Validate quantity (1-125 for registers)
        if quantity == 0 || quantity > 125 {
            return Some(vec![function_code | 0x80, 0x03]); // Illegal Data Value
        }

        // Check address range
        if start_address + quantity > 256 {
            return Some(vec![function_code | 0x80, 0x02]); // Illegal Data Address
        }

        let storage = storage.lock().ok()?;
        let registers = if is_holding {
            &storage.holding_registers[start_address..start_address + quantity]
        } else {
            &storage.input_registers[start_address..start_address + quantity]
        };

        let byte_count = quantity * 2;
        let mut response = Vec::with_capacity(2 + byte_count);
        response.push(function_code);
        response.push(byte_count as u8);

        for &reg in registers {
            response.extend_from_slice(&reg.to_be_bytes());
        }

        Some(response)
    }

    /// Handle Write Single Coil (0x05).
    fn handle_write_single_coil(pdu: &[u8], storage: &SharedStorage) -> Option<Vec<u8>> {
        if pdu.len() < 5 {
            return Some(vec![pdu[0] | 0x80, 0x03]); // Illegal Data Value
        }

        let function_code = pdu[0];
        let address = u16::from_be_bytes([pdu[1], pdu[2]]) as usize;
        let value = u16::from_be_bytes([pdu[3], pdu[4]]);

        // Validate address
        if address >= 256 {
            return Some(vec![function_code | 0x80, 0x02]); // Illegal Data Address
        }

        // Validate value (must be 0x0000 or 0xFF00)
        if value != 0x0000 && value != 0xFF00 {
            return Some(vec![function_code | 0x80, 0x03]); // Illegal Data Value
        }

        // Write the coil
        if let Ok(mut s) = storage.lock() {
            s.coils[address] = value == 0xFF00;
        }

        // Response echoes the request
        Some(pdu.to_vec())
    }

    /// Handle Write Single Register (0x06).
    fn handle_write_single_register(pdu: &[u8], storage: &SharedStorage) -> Option<Vec<u8>> {
        if pdu.len() < 5 {
            return Some(vec![pdu[0] | 0x80, 0x03]); // Illegal Data Value
        }

        let function_code = pdu[0];
        let address = u16::from_be_bytes([pdu[1], pdu[2]]) as usize;
        let value = u16::from_be_bytes([pdu[3], pdu[4]]);

        // Validate address
        if address >= 256 {
            return Some(vec![function_code | 0x80, 0x02]); // Illegal Data Address
        }

        // Write the register
        if let Ok(mut s) = storage.lock() {
            s.holding_registers[address] = value;
        }

        // Response echoes the request
        Some(pdu.to_vec())
    }

    /// Handle Write Multiple Coils (0x0F).
    fn handle_write_multiple_coils(pdu: &[u8], storage: &SharedStorage) -> Option<Vec<u8>> {
        if pdu.len() < 6 {
            return Some(vec![pdu[0] | 0x80, 0x03]); // Illegal Data Value
        }

        let function_code = pdu[0];
        let start_address = u16::from_be_bytes([pdu[1], pdu[2]]) as usize;
        let quantity = u16::from_be_bytes([pdu[3], pdu[4]]) as usize;
        let byte_count = pdu[5] as usize;

        // Validate quantity
        if quantity == 0 || quantity > 1968 {
            return Some(vec![function_code | 0x80, 0x03]); // Illegal Data Value
        }

        // Validate byte count
        let expected_bytes = (quantity + 7) / 8;
        if byte_count != expected_bytes || pdu.len() < 6 + byte_count {
            return Some(vec![function_code | 0x80, 0x03]); // Illegal Data Value
        }

        // Check address range
        if start_address + quantity > 256 {
            return Some(vec![function_code | 0x80, 0x02]); // Illegal Data Address
        }

        // Write coils
        if let Ok(mut s) = storage.lock() {
            for i in 0..quantity {
                let byte_idx = i / 8;
                let bit_idx = i % 8;
                let bit = (pdu[6 + byte_idx] >> bit_idx) & 1 != 0;
                s.coils[start_address + i] = bit;
            }
        }

        // Response: function code, start address, quantity
        let mut response = Vec::with_capacity(5);
        response.push(function_code);
        response.extend_from_slice(&(start_address as u16).to_be_bytes());
        response.extend_from_slice(&(quantity as u16).to_be_bytes());

        Some(response)
    }

    /// Handle Write Multiple Registers (0x10).
    fn handle_write_multiple_registers(pdu: &[u8], storage: &SharedStorage) -> Option<Vec<u8>> {
        if pdu.len() < 6 {
            return Some(vec![pdu[0] | 0x80, 0x03]); // Illegal Data Value
        }

        let function_code = pdu[0];
        let start_address = u16::from_be_bytes([pdu[1], pdu[2]]) as usize;
        let quantity = u16::from_be_bytes([pdu[3], pdu[4]]) as usize;
        let byte_count = pdu[5] as usize;

        // Validate quantity
        if quantity == 0 || quantity > 123 {
            return Some(vec![function_code | 0x80, 0x03]); // Illegal Data Value
        }

        // Validate byte count
        let expected_bytes = quantity * 2;
        if byte_count != expected_bytes || pdu.len() < 6 + byte_count {
            return Some(vec![function_code | 0x80, 0x03]); // Illegal Data Value
        }

        // Check address range
        if start_address + quantity > 256 {
            return Some(vec![function_code | 0x80, 0x02]); // Illegal Data Address
        }

        // Write registers
        if let Ok(mut s) = storage.lock() {
            for i in 0..quantity {
                let offset = 6 + i * 2;
                let value = u16::from_be_bytes([pdu[offset], pdu[offset + 1]]);
                s.holding_registers[start_address + i] = value;
            }
        }

        // Response: function code, start address, quantity
        let mut response = Vec::with_capacity(5);
        response.push(function_code);
        response.extend_from_slice(&(start_address as u16).to_be_bytes());
        response.extend_from_slice(&(quantity as u16).to_be_bytes());

        Some(response)
    }
}

impl Drop for MockModbusServer {
    fn drop(&mut self) {
        self.stop_signal.store(true, Ordering::SeqCst);
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;

    /// Helper to send a Modbus TCP request and read the response.
    fn send_request(stream: &mut TcpStream, unit_id: u8, pdu: &[u8]) -> Vec<u8> {
        static TRANSACTION_ID: std::sync::atomic::AtomicU16 =
            std::sync::atomic::AtomicU16::new(0);

        let transaction_id = TRANSACTION_ID.fetch_add(1, Ordering::SeqCst);

        // Build MBAP header
        let mut request = Vec::with_capacity(7 + pdu.len());
        request.extend_from_slice(&transaction_id.to_be_bytes());
        request.extend_from_slice(&0u16.to_be_bytes()); // Protocol ID
        request.extend_from_slice(&((pdu.len() + 1) as u16).to_be_bytes()); // Length
        request.push(unit_id);
        request.extend_from_slice(pdu);

        stream.write_all(&request).unwrap();

        // Read response header
        let mut header = [0u8; 7];
        stream.read_exact(&mut header).unwrap();

        let resp_length = u16::from_be_bytes([header[4], header[5]]) as usize;
        let pdu_length = resp_length.saturating_sub(1);

        // Read response PDU
        let mut response = vec![0u8; pdu_length];
        if pdu_length > 0 {
            stream.read_exact(&mut response).unwrap();
        }

        response
    }

    #[test]
    fn test_server_starts_and_binds() {
        let server = MockModbusServer::start(MockBehavior::Normal).unwrap();
        let addr = server.local_addr();

        assert!(addr.port() > 0);
        assert_eq!(addr.ip().to_string(), "127.0.0.1");

        server.stop();
    }

    #[test]
    fn test_read_coils() {
        let server = MockModbusServer::start(MockBehavior::Normal).unwrap();

        // Set some coils
        server.set_coil(0, true);
        server.set_coil(1, false);
        server.set_coil(2, true);
        server.set_coil(7, true);

        let mut stream = TcpStream::connect(server.local_addr()).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

        // Read Coils: FC=0x01, Start=0, Quantity=8
        let pdu = [0x01, 0x00, 0x00, 0x00, 0x08];
        let response = send_request(&mut stream, 1, &pdu);

        assert_eq!(response[0], 0x01); // Function code
        assert_eq!(response[1], 1); // Byte count
        // Bits: 0=1, 1=0, 2=1, 7=1 => 0b10000101 = 0x85
        assert_eq!(response[2], 0x85);

        server.stop();
    }

    #[test]
    fn test_read_discrete_inputs() {
        let server = MockModbusServer::start(MockBehavior::Normal).unwrap();

        server.set_discrete_input(0, true);
        server.set_discrete_input(3, true);

        let mut stream = TcpStream::connect(server.local_addr()).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

        // Read Discrete Inputs: FC=0x02, Start=0, Quantity=8
        let pdu = [0x02, 0x00, 0x00, 0x00, 0x08];
        let response = send_request(&mut stream, 1, &pdu);

        assert_eq!(response[0], 0x02);
        assert_eq!(response[1], 1);
        // Bits: 0=1, 3=1 => 0b00001001 = 0x09
        assert_eq!(response[2], 0x09);

        server.stop();
    }

    #[test]
    fn test_read_holding_registers() {
        let server = MockModbusServer::start(MockBehavior::Normal).unwrap();

        server.set_holding_register(0, 0x1234);
        server.set_holding_register(1, 0x5678);

        let mut stream = TcpStream::connect(server.local_addr()).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

        // Read Holding Registers: FC=0x03, Start=0, Quantity=2
        let pdu = [0x03, 0x00, 0x00, 0x00, 0x02];
        let response = send_request(&mut stream, 1, &pdu);

        assert_eq!(response[0], 0x03);
        assert_eq!(response[1], 4); // 2 registers * 2 bytes
        assert_eq!(response[2], 0x12);
        assert_eq!(response[3], 0x34);
        assert_eq!(response[4], 0x56);
        assert_eq!(response[5], 0x78);

        server.stop();
    }

    #[test]
    fn test_read_input_registers() {
        let server = MockModbusServer::start(MockBehavior::Normal).unwrap();

        server.set_input_register(0, 0xABCD);

        let mut stream = TcpStream::connect(server.local_addr()).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

        // Read Input Registers: FC=0x04, Start=0, Quantity=1
        let pdu = [0x04, 0x00, 0x00, 0x00, 0x01];
        let response = send_request(&mut stream, 1, &pdu);

        assert_eq!(response[0], 0x04);
        assert_eq!(response[1], 2);
        assert_eq!(response[2], 0xAB);
        assert_eq!(response[3], 0xCD);

        server.stop();
    }

    #[test]
    fn test_write_single_coil() {
        let server = MockModbusServer::start(MockBehavior::Normal).unwrap();

        let mut stream = TcpStream::connect(server.local_addr()).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

        // Write Single Coil: FC=0x05, Address=5, Value=ON (0xFF00)
        let pdu = [0x05, 0x00, 0x05, 0xFF, 0x00];
        let response = send_request(&mut stream, 1, &pdu);

        // Response echoes request
        assert_eq!(response, pdu);

        // Verify coil was set
        assert!(server.get_coil(5));

        server.stop();
    }

    #[test]
    fn test_write_single_register() {
        let server = MockModbusServer::start(MockBehavior::Normal).unwrap();

        let mut stream = TcpStream::connect(server.local_addr()).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

        // Write Single Register: FC=0x06, Address=10, Value=0x1234
        let pdu = [0x06, 0x00, 0x0A, 0x12, 0x34];
        let response = send_request(&mut stream, 1, &pdu);

        // Response echoes request
        assert_eq!(response, pdu);

        // Verify register was set
        assert_eq!(server.get_holding_register(10), 0x1234);

        server.stop();
    }

    #[test]
    fn test_write_multiple_coils() {
        let server = MockModbusServer::start(MockBehavior::Normal).unwrap();

        let mut stream = TcpStream::connect(server.local_addr()).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

        // Write Multiple Coils: FC=0x0F, Start=0, Quantity=8, Byte count=1, Data=0xAA
        // 0xAA = 10101010 = coils 1,3,5,7 ON
        let pdu = [0x0F, 0x00, 0x00, 0x00, 0x08, 0x01, 0xAA];
        let response = send_request(&mut stream, 1, &pdu);

        assert_eq!(response[0], 0x0F);
        assert_eq!(response.len(), 5);

        // Verify coils
        assert!(!server.get_coil(0)); // Bit 0 = 0
        assert!(server.get_coil(1)); // Bit 1 = 1
        assert!(!server.get_coil(2)); // Bit 2 = 0
        assert!(server.get_coil(3)); // Bit 3 = 1
        assert!(!server.get_coil(4));
        assert!(server.get_coil(5));
        assert!(!server.get_coil(6));
        assert!(server.get_coil(7));

        server.stop();
    }

    #[test]
    fn test_write_multiple_registers() {
        let server = MockModbusServer::start(MockBehavior::Normal).unwrap();

        let mut stream = TcpStream::connect(server.local_addr()).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

        // Write Multiple Registers: FC=0x10, Start=0, Quantity=2, Bytes=4, Data=0x1234, 0x5678
        let pdu = [0x10, 0x00, 0x00, 0x00, 0x02, 0x04, 0x12, 0x34, 0x56, 0x78];
        let response = send_request(&mut stream, 1, &pdu);

        assert_eq!(response[0], 0x10);
        assert_eq!(response.len(), 5);

        // Verify registers
        assert_eq!(server.get_holding_register(0), 0x1234);
        assert_eq!(server.get_holding_register(1), 0x5678);

        server.stop();
    }

    #[test]
    fn test_exception_behavior() {
        let server = MockModbusServer::start(MockBehavior::Exception(0x02)).unwrap();

        let mut stream = TcpStream::connect(server.local_addr()).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

        // Any request should get exception response
        let pdu = [0x03, 0x00, 0x00, 0x00, 0x01];
        let response = send_request(&mut stream, 1, &pdu);

        // Exception: FC | 0x80, exception code
        assert_eq!(response[0], 0x83); // 0x03 | 0x80
        assert_eq!(response[1], 0x02); // Illegal Data Address

        server.stop();
    }

    #[test]
    fn test_illegal_function() {
        let server = MockModbusServer::start(MockBehavior::Normal).unwrap();

        let mut stream = TcpStream::connect(server.local_addr()).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

        // Unknown function code 0x99
        let pdu = [0x99, 0x00, 0x00];
        let response = send_request(&mut stream, 1, &pdu);

        assert_eq!(response[0], 0x99 | 0x80); // Exception
        assert_eq!(response[1], 0x01); // Illegal Function

        server.stop();
    }

    #[test]
    fn test_illegal_data_address() {
        let server = MockModbusServer::start(MockBehavior::Normal).unwrap();

        let mut stream = TcpStream::connect(server.local_addr()).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

        // Read Holding Registers with out-of-range address (start=250, qty=10 > 256)
        let pdu = [0x03, 0x00, 0xFA, 0x00, 0x0A];
        let response = send_request(&mut stream, 1, &pdu);

        assert_eq!(response[0], 0x83); // Exception
        assert_eq!(response[1], 0x02); // Illegal Data Address

        server.stop();
    }

    #[test]
    fn test_behavior_change_at_runtime() {
        let server = MockModbusServer::start(MockBehavior::Normal).unwrap();

        assert_eq!(server.behavior(), MockBehavior::Normal);

        server.set_behavior(MockBehavior::Exception(0x04));
        assert_eq!(server.behavior(), MockBehavior::Exception(0x04));

        // Now requests should get exception
        let mut stream = TcpStream::connect(server.local_addr()).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

        let pdu = [0x03, 0x00, 0x00, 0x00, 0x01];
        let response = send_request(&mut stream, 1, &pdu);

        assert_eq!(response[0], 0x83);
        assert_eq!(response[1], 0x04);

        server.stop();
    }

    #[test]
    fn test_storage_access() {
        let server = MockModbusServer::start(MockBehavior::Normal).unwrap();

        // Test direct storage access
        server.with_storage(|s| {
            s.coils[100] = true;
            s.holding_registers[50] = 0xBEEF;
        });

        assert!(server.get_coil(100));
        assert_eq!(server.get_holding_register(50), 0xBEEF);

        server.stop();
    }

    #[test]
    fn test_initial_storage() {
        let mut initial = ModbusStorage::default();
        initial.coils[0] = true;
        initial.holding_registers[0] = 0xCAFE;

        let server =
            MockModbusServer::start_with_storage(MockBehavior::Normal, initial).unwrap();

        assert!(server.get_coil(0));
        assert_eq!(server.get_holding_register(0), 0xCAFE);

        server.stop();
    }

    #[test]
    fn test_concurrent_connections() {
        let server = MockModbusServer::start(MockBehavior::Normal).unwrap();
        server.set_holding_register(0, 42);

        let addr = server.local_addr();

        // Spawn multiple client threads
        let handles: Vec<_> = (0..3)
            .map(|_| {
                let addr = addr;
                thread::spawn(move || {
                    let mut stream = TcpStream::connect(addr).unwrap();
                    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

                    let pdu = [0x03, 0x00, 0x00, 0x00, 0x01];
                    let response = send_request(&mut stream, 1, &pdu);

                    assert_eq!(response[0], 0x03);
                    assert_eq!(response[1], 2);
                    u16::from_be_bytes([response[2], response[3]])
                })
            })
            .collect();

        for handle in handles {
            let value = handle.join().unwrap();
            assert_eq!(value, 42);
        }

        server.stop();
    }
}
