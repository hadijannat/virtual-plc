use plc_common::PlcResult;

/// Modbus TCP driver (scaffold).
pub struct ModbusTcpDriver;

impl ModbusTcpDriver {
    pub fn new() -> Self {
        Self
    }
    pub fn init(&mut self) -> PlcResult<()> {
        Ok(())
    }
}
