//! Fieldbus plane abstractions for industrial communication.
//!
//! This crate provides:
//! - [`FieldbusDriver`] trait for abstracting fieldbus communication
//! - [`ethercat`] module with EtherCAT master implementation
//! - [`modbus`] module with Modbus TCP support (scaffold)
//! - [`slave_config`] module with EtherCAT slave configuration
//! - [`dc_sync`] module with Distributed Clocks synchronization

pub mod dc_sync;
pub mod ethercat;
pub mod modbus;
pub mod slave_config;

pub use dc_sync::*;
pub use ethercat::*;
pub use slave_config::*;

use plc_common::PlcResult;

/// Fieldbus driver abstraction.
///
/// This trait defines the interface for all fieldbus drivers,
/// allowing the runtime to work with different fieldbus types
/// (EtherCAT, Modbus TCP, etc.) through a common interface.
pub trait FieldbusDriver: Send {
    /// Initialize the fieldbus driver.
    ///
    /// This should:
    /// - Open the network interface
    /// - Scan for slaves/devices
    /// - Configure slave parameters
    /// - Transition to operational state
    fn init(&mut self) -> PlcResult<()>;

    /// Read inputs from the fieldbus.
    ///
    /// Called before the logic engine executes to update
    /// the input portion of the process image.
    fn read_inputs(&mut self) -> PlcResult<()>;

    /// Write outputs to the fieldbus.
    ///
    /// Called after the logic engine executes to send
    /// the output portion of the process image.
    fn write_outputs(&mut self) -> PlcResult<()>;

    /// Perform a combined exchange cycle.
    ///
    /// For protocols like EtherCAT that send/receive in a single frame,
    /// this is more efficient than separate read/write calls.
    /// Default implementation calls read_inputs then write_outputs.
    fn exchange(&mut self) -> PlcResult<()> {
        self.read_inputs()?;
        self.write_outputs()
    }

    /// Shutdown the fieldbus driver gracefully.
    ///
    /// Should transition slaves to a safe state and release resources.
    fn shutdown(&mut self) -> PlcResult<()>;

    /// Check if the driver is operational.
    fn is_operational(&self) -> bool {
        true
    }
}

/// Supported fieldbus driver types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverKind {
    /// EtherCAT via SOEM or simulated.
    EtherCAT,
    /// Modbus TCP.
    ModbusTcp,
    /// Simulated I/O for testing.
    Simulated,
}

/// Simulated fieldbus driver for testing.
///
/// Provides a no-op implementation that always succeeds.
#[derive(Debug, Default)]
pub struct SimulatedDriver {
    initialized: bool,
}

impl SimulatedDriver {
    /// Create a new simulated driver.
    pub fn new() -> Self {
        Self { initialized: false }
    }
}

impl FieldbusDriver for SimulatedDriver {
    fn init(&mut self) -> PlcResult<()> {
        self.initialized = true;
        Ok(())
    }

    fn read_inputs(&mut self) -> PlcResult<()> {
        Ok(())
    }

    fn write_outputs(&mut self) -> PlcResult<()> {
        Ok(())
    }

    fn shutdown(&mut self) -> PlcResult<()> {
        self.initialized = false;
        Ok(())
    }

    fn is_operational(&self) -> bool {
        self.initialized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simulated_driver() {
        let mut driver = SimulatedDriver::new();
        assert!(!driver.is_operational());

        driver.init().unwrap();
        assert!(driver.is_operational());

        driver.exchange().unwrap();

        driver.shutdown().unwrap();
        assert!(!driver.is_operational());
    }
}
