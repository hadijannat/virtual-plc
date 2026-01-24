# Virtual PLC (vPLC)

A production-grade soft PLC runtime in Rust, targeting real-time industrial control with WebAssembly-sandboxed logic execution.

![Build Status](https://img.shields.io/badge/build-passing-brightgreen)

## Overview

vPLC implements a **split-plane architecture** that decouples fieldbus I/O from logic execution:

- **Fieldbus Plane**: Handles real-time communication with industrial devices (EtherCAT, Modbus TCP, simulated I/O)
- **Logic Plane**: Executes IEC 61131-3 Structured Text programs compiled to WebAssembly

This separation provides fault isolation, deterministic timing, and the ability to hot-reload logic without disrupting I/O communication.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        plc-daemon                           │
│              (Binary entry point, signal handling)          │
└─────────────────────────┬───────────────────────────────────┘
                          │
          ┌───────────────┼───────────────┐
          │               │               │
          ▼               ▼               ▼
┌─────────────────┐ ┌───────────┐ ┌─────────────────┐
│   plc-runtime   │ │plc-compiler│ │   plc-web-ui   │
│ Scheduler, Wasm │ │ ST → Wasm │ │ (Control plane) │
│ host, I/O image │ │  pipeline │ │                 │
└────────┬────────┘ └─────┬─────┘ └─────────────────┘
         │                │
         ▼                ▼
┌─────────────────┐ ┌───────────┐
│  plc-fieldbus   │ │ plc-stdlib │
│ EtherCAT/Modbus │ │  FB library│
└────────┬────────┘ └─────┬─────┘
         │                │
         └───────┬────────┘
                 ▼
        ┌─────────────────┐
        │   plc-common    │
        │ Types, config,  │
        │ error handling  │
        └─────────────────┘
```

### Crates

| Crate | Description |
|-------|-------------|
| `plc-daemon` | Binary entry point with signal handling and diagnostics |
| `plc-runtime` | Cyclic scheduler, Wasm host (Wasmtime), process image |
| `plc-compiler` | IEC 61131-3 Structured Text → WebAssembly compiler |
| `plc-fieldbus` | Fieldbus abstraction (EtherCAT, Modbus TCP, simulated) |
| `plc-stdlib` | Standard function blocks (timers, counters, triggers) |
| `plc-common` | Shared IEC types, configuration, error handling |
| `plc-web-ui` | Control plane web interface (scaffold) |

## Quick Start

### Build

```bash
# Build all crates
cargo build --release

# Run tests
cargo test -q
```

### Run with Simulated I/O

```bash
# Start daemon with simulated fieldbus
cargo run -p plc-daemon -- --simulated

# With a compiled Wasm module
cargo run -p plc-daemon -- --simulated -w programs/blink.wasm
```

### Compile ST to Wasm

```rust
use plc_compiler::compile;

let source = r#"
    PROGRAM Main
    VAR
        counter : INT := 0;
    END_VAR
        counter := counter + 1;
    END_PROGRAM
"#;

let wasm_bytes = compile(source).expect("Compilation failed");
```

## Configuration

### CLI Flags

| Flag | Description |
|------|-------------|
| `-c, --config <FILE>` | Path to TOML configuration file |
| `-w, --wasm-module <FILE>` | Path to Wasm logic module |
| `-s, --simulated` | Use simulated fieldbus (no hardware) |
| `--max-cycles <N>` | Maximum cycles to run (0 = infinite) |
| `-l, --log-level <LEVEL>` | Log level (trace, debug, info, warn, error) |

### TOML Configuration

```toml
# Scan cycle time
cycle_time = "1ms"
watchdog_timeout = "3ms"
max_overrun = "500us"

# Wasm logic module
wasm_module = "programs/main.wasm"

[realtime]
enabled = true
policy = "fifo"           # fifo, rr, or other
priority = 90             # 1-99 for RT policies
cpu_affinity = 2          # Pin to CPU core 2
lock_memory = true
prefault_stack_size = 8388608

[fieldbus]
driver = "ethercat"       # simulated, ethercat, modbus_tcp

[fieldbus.ethercat]
interface = "eth0"
dc_enabled = true
dc_sync0_cycle = "1ms"

[metrics]
enabled = true
histogram_size = 10000
percentiles = [50.0, 90.0, 99.0, 99.9]
http_export = false
http_port = 9090
```

See [`config/default.toml`](config/default.toml) for a fully documented example.

## IEC 61131-3 Support

### Data Types

| Type | Rust Equivalent | Description |
|------|-----------------|-------------|
| `BOOL` | `bool` | Boolean |
| `SINT`, `INT`, `DINT`, `LINT` | `i8`, `i16`, `i32`, `i64` | Signed integers |
| `USINT`, `UINT`, `UDINT`, `ULINT` | `u8`, `u16`, `u32`, `u64` | Unsigned integers |
| `REAL`, `LREAL` | `f32`, `f64` | Floating point |
| `BYTE`, `WORD`, `DWORD`, `LWORD` | `u8`, `u16`, `u32`, `u64` | Bit strings |
| `TIME` | `i64` (nanoseconds) | Duration |
| `STRING`, `WSTRING` | String | Character strings |
| `ARRAY[l..u] OF T` | Array | Arrays with bounds |

### Program Units

- `PROGRAM` - Main program unit
- `FUNCTION_BLOCK` - Reusable stateful blocks
- `FUNCTION` - Stateless functions with return values

### Control Flow

- `IF ... THEN ... ELSIF ... ELSE ... END_IF`
- `CASE ... OF ... ELSE ... END_CASE`
- `FOR ... TO ... BY ... DO ... END_FOR`
- `WHILE ... DO ... END_WHILE`
- `REPEAT ... UNTIL ... END_REPEAT`
- `EXIT`, `CONTINUE`, `RETURN`

### Standard Function Blocks (plc-stdlib)

| Block | Description |
|-------|-------------|
| `TON` | Timer On-Delay |
| `TOF` | Timer Off-Delay |
| `TP` | Timer Pulse |
| `CTU` | Counter Up |
| `CTD` | Counter Down |
| `CTUD` | Counter Up/Down |
| `R_TRIG` | Rising Edge Trigger |
| `F_TRIG` | Falling Edge Trigger |
| `SR` | Set-Reset Flip-Flop (set dominant) |
| `RS` | Reset-Set Flip-Flop (reset dominant) |

## Real-Time Deployment

### Requirements

For deterministic real-time execution:

1. **PREEMPT_RT Kernel**: Linux kernel with `PREEMPT_RT` patches
2. **CPU Isolation**: Dedicate CPU cores to PLC tasks via `isolcpus`
3. **Privileges**: `CAP_SYS_NICE`, `CAP_IPC_LOCK`, `CAP_NET_RAW`

### Docker Deployment

```yaml
# docker-compose.yml
services:
  plc:
    build: .
    privileged: true
    cap_add:
      - SYS_NICE
      - IPC_LOCK
      - NET_RAW
    ulimits:
      rtprio: 99
      memlock: -1
    network_mode: host
```

```bash
docker compose build
docker compose up
```

### Host Tuning

See [`scripts/host_tune.md`](scripts/host_tune.md) for kernel parameter tuning.

### Latency Verification

```bash
./scripts/verify_latency.sh
```

Uses `cyclictest` to measure scheduling latency. Target: < 50µs worst-case.

## Wasm Host API

Wasm modules import functions from the `"plc"` module:

| Function | Signature | Description |
|----------|-----------|-------------|
| `read_di` | `(i32) -> i32` | Read digital input bit |
| `write_do` | `(i32, i32) -> ()` | Write digital output bit |
| `read_ai` | `(i32) -> i32` | Read analog input channel |
| `write_ao` | `(i32, i32) -> ()` | Write analog output channel |
| `get_cycle_time` | `() -> i32` | Get cycle time in nanoseconds |
| `get_cycle_count` | `() -> i64` | Get current cycle number |
| `is_first_cycle` | `() -> i32` | Check if first cycle after init |
| `log_message` | `(i32, i32) -> ()` | Log message (ptr, len) |

### Module Requirements

Wasm modules must export:
- `memory` - Linear memory
- `step()` - Called every scan cycle

Optional exports:
- `init()` - Called once before first cycle
- `fault()` - Called when entering fault mode

## Key Abstractions

### LogicEngine Trait

```rust
pub trait LogicEngine: Send {
    fn init(&mut self) -> PlcResult<()>;
    fn step(&mut self, inputs: &ProcessData) -> PlcResult<ProcessData>;
    fn fault(&mut self) -> PlcResult<()>;
    fn is_ready(&self) -> bool;
}
```

Implementations: `WasmtimeHost`, `NullEngine`

### FieldbusDriver Trait

```rust
pub trait FieldbusDriver: Send {
    fn init(&mut self) -> PlcResult<()>;
    fn read_inputs(&mut self) -> PlcResult<()>;
    fn write_outputs(&mut self) -> PlcResult<()>;
    fn exchange(&mut self) -> PlcResult<()>;
    fn shutdown(&mut self) -> PlcResult<()>;
}
```

Implementations: `SimulatedDriver`, `EtherCatDriver` (scaffold)

## Documentation

- [`docs/adr/001-runtime-arch.md`](docs/adr/001-runtime-arch.md) - Runtime architecture decisions
- [`docs/adr/002-wasm-sandbox.md`](docs/adr/002-wasm-sandbox.md) - Wasm sandboxing rationale
- [`docs/adr/003-ethercat-primary.md`](docs/adr/003-ethercat-primary.md) - EtherCAT as primary fieldbus
- [`docs/architecture.md`](docs/architecture.md) - Detailed architecture overview
- [`docs/acceptance-criteria.md`](docs/acceptance-criteria.md) - Production acceptance criteria
- [`scripts/host_tune.md`](scripts/host_tune.md) - Host tuning guide

## License

MIT OR Apache-2.0
