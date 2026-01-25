# plc-fieldbus

Industrial fieldbus communication for Virtual PLC.

## Features

- **Modbus TCP** - Full client implementation (FC 0x01-0x06, 0x0F-0x10)
- **EtherCAT** - Master with DC support (SOEM backend or simulated)
- **Simulated** - In-memory I/O for testing

## Quick Start

```rust
use plc_fieldbus::{SimulatedDriver, FieldbusDriver};

let mut driver = SimulatedDriver::new();
driver.init()?;
driver.exchange()?;
driver.shutdown()?;
```

## Modbus TCP Example

```rust
use plc_fieldbus::{ModbusTcpDriver, ModbusTcpConfig, FieldbusDriver};

let config = ModbusTcpConfig {
    server_addr: "192.168.1.100:502".parse()?,
    unit_id: 1,
    ..Default::default()
};
let mut driver = ModbusTcpDriver::with_config(config);
driver.init()?;

// Set outputs before exchange
let outputs = plc_fieldbus::FieldbusOutputs {
    digital: 0xFF,
    analog: [100, 200, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
};
driver.set_outputs(&outputs);

// Perform I/O exchange
driver.exchange()?;

// Read inputs after exchange
let inputs = driver.get_inputs();
println!("Digital inputs: 0x{:08X}", inputs.digital);

driver.shutdown()?;
```

## EtherCAT Example

```rust
use plc_fieldbus::{EthercatMaster, FieldbusDriver};
use plc_common::config::EthercatConfig;
use std::time::Duration;

let config = EthercatConfig {
    interface: Some("eth0".into()),
    dc_enabled: true,
    dc_sync0_cycle: Duration::from_millis(1),
    esi_path: None,
    wkc_error_threshold: 3,
};
let mut master = EthercatMaster::new(config);
master.init()?;

// Cyclic exchange loop
loop {
    master.exchange()?;

    let inputs = master.get_inputs();
    // Process inputs, compute outputs...

    std::thread::sleep(Duration::from_millis(1));
}
```

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `simulated` | Yes | Simulated drivers for testing without hardware |
| `soem` | No | Real EtherCAT via SOEM (Linux only, requires libsoem) |

Enable SOEM support:

```toml
[dependencies]
plc-fieldbus = { version = "0.1", features = ["soem"] }
```

## FieldbusDriver Trait

All drivers implement the `FieldbusDriver` trait:

```rust
pub trait FieldbusDriver: Send {
    fn init(&mut self) -> PlcResult<()>;
    fn read_inputs(&mut self) -> PlcResult<()>;
    fn write_outputs(&mut self) -> PlcResult<()>;
    fn exchange(&mut self) -> PlcResult<()>;  // Combined read+write
    fn get_inputs(&self) -> FieldbusInputs;
    fn set_outputs(&mut self, outputs: &FieldbusOutputs);
    fn shutdown(&mut self) -> PlcResult<()>;
    fn is_operational(&self) -> bool;
}
```

## Modbus Function Codes

| Code | Function |
|------|----------|
| 0x01 | Read Coils |
| 0x02 | Read Discrete Inputs |
| 0x03 | Read Holding Registers |
| 0x04 | Read Input Registers |
| 0x05 | Write Single Coil |
| 0x06 | Write Single Register |
| 0x0F | Write Multiple Coils |
| 0x10 | Write Multiple Registers |

## Troubleshooting

### Modbus TCP

**Connection refused**: Verify the server address and port. Default Modbus port is 502.

**Timeout errors**: Increase `io_timeout` in `ModbusTcpConfig`. Default is 1 second.

**Exception responses**: Check the Modbus exception code in the error message:
- `Illegal Function`: Function code not supported by device
- `Illegal Data Address`: Register address out of range
- `Illegal Data Value`: Value outside acceptable range

### EtherCAT

**No slaves found**: Verify network interface name and cable connections. On Linux, check with `ip link show`.

**Working counter mismatch**: A slave may have dropped off the network. Check physical connections and slave power.

**Permission denied**: EtherCAT requires raw socket access. Run as root or grant `CAP_NET_RAW`:

```bash
sudo setcap cap_net_raw+ep ./target/release/plc-daemon
```

**DC synchronization issues**: Ensure all slaves support DC and are properly configured. Check `dc_enabled` in config.

## Requirements

### Modbus TCP
- Network access to Modbus server

### EtherCAT (with `soem` feature)
- Linux with raw socket support
- SOEM library installed (`libsoem-dev` on Debian/Ubuntu)
- Root privileges or `CAP_NET_RAW` capability
- Dedicated network interface (recommended)
- PREEMPT_RT kernel for deterministic timing (production)

## License

See the workspace root LICENSE file.
