# Virtual PLC Architecture

This document provides a deep technical overview of the Virtual PLC architecture, design decisions, and implementation details.

## Design Philosophy

Virtual PLC is built on three core principles:

1. **Safety through isolation**: User logic cannot crash the I/O system
2. **Determinism**: Predictable, bounded cycle times
3. **Portability**: Same logic runs on any platform supporting Wasm

## Split-Plane Architecture

The runtime separates concerns into two distinct planes:

```
┌─────────────────────────────────────────────────────────────┐
│                      Control Plane                          │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐ │
│  │   Web UI    │  │  REST API   │  │  Prometheus Metrics │ │
│  └──────┬──────┘  └──────┬──────┘  └──────────┬──────────┘ │
│         │                │                     │            │
│         └────────────────┼─────────────────────┘            │
│                          │                                  │
│                    ┌─────▼─────┐                            │
│                    │  Shared   │                            │
│                    │   State   │                            │
│                    └─────┬─────┘                            │
└──────────────────────────┼──────────────────────────────────┘
                           │
┌──────────────────────────┼──────────────────────────────────┐
│                    Logic Plane                              │
│                    ┌─────▼─────┐                            │
│                    │ Scheduler │                            │
│                    └─────┬─────┘                            │
│         ┌────────────────┼────────────────┐                 │
│         │                │                │                 │
│    ┌────▼────┐     ┌─────▼─────┐    ┌─────▼─────┐          │
│    │  Wasm   │     │  Process  │    │   Fault   │          │
│    │  Host   │     │   Image   │    │  Recorder │          │
│    └────┬────┘     └─────┬─────┘    └───────────┘          │
│         │                │                                  │
│    ┌────▼────┐           │                                  │
│    │  User   │           │                                  │
│    │  Logic  │           │                                  │
│    └─────────┘           │                                  │
└──────────────────────────┼──────────────────────────────────┘
                           │
┌──────────────────────────┼──────────────────────────────────┐
│                   Fieldbus Plane                            │
│                    ┌─────▼─────┐                            │
│                    │ Fieldbus  │                            │
│                    │  Manager  │                            │
│                    └─────┬─────┘                            │
│         ┌────────────────┼────────────────┐                 │
│         │                │                │                 │
│    ┌────▼────┐     ┌─────▼─────┐    ┌─────▼─────┐          │
│    │EtherCAT │     │  Modbus   │    │  Simulated│          │
│    │ Driver  │     │   TCP     │    │    I/O    │          │
│    └─────────┘     └───────────┘    └───────────┘          │
└─────────────────────────────────────────────────────────────┘
```

### Why Split Planes?

| Benefit | Description |
|---------|-------------|
| **Fault Isolation** | A bug in user logic cannot corrupt I/O state or crash the fieldbus driver |
| **Independent Updates** | Hot-reload logic without stopping I/O communication |
| **Security Boundary** | Wasm sandbox prevents logic from accessing system resources |
| **Testability** | Each plane can be tested independently |

## Crate Dependency Graph

```
plc-daemon (binary entry point)
├── plc-runtime
│   ├── plc-fieldbus
│   │   └── plc-common
│   └── plc-common
├── plc-compiler
│   └── plc-common
├── plc-web-ui
│   └── plc-common
└── plc-stdlib
    └── plc-common
```

### Crate Responsibilities

| Crate | Purpose |
|-------|---------|
| `plc-daemon` | CLI interface, signal handling, daemon lifecycle |
| `plc-runtime` | Scheduler, Wasm host, I/O image, fault recording |
| `plc-compiler` | IEC 61131-3 ST parser, IR, Wasm codegen |
| `plc-fieldbus` | EtherCAT, Modbus TCP, simulated I/O drivers |
| `plc-web-ui` | REST API, WebSocket streaming, Prometheus metrics |
| `plc-common` | IEC types, error types, runtime state enum |
| `plc-stdlib` | Standard function blocks (TON, CTU, etc.) |

## Key Abstractions

### LogicEngine Trait

The `LogicEngine` trait (`plc-runtime/src/wasm_host.rs`) defines the interface for executing user logic:

```rust
pub trait LogicEngine: Send {
    /// Initialize the logic engine (called once at startup)
    fn init(&mut self) -> PlcResult<()>;

    /// Execute one scan cycle
    fn step(&mut self, inputs: &ProcessData) -> PlcResult<ProcessData>;

    /// Handle fault condition (cleanup, safe state)
    fn fault(&mut self) -> PlcResult<()>;

    /// Check if engine is ready to execute
    fn is_ready(&self) -> bool;

    /// Hot-reload with new Wasm module (optional)
    fn reload_module(&mut self, wasm_bytes: &[u8], preserve_memory: bool) -> PlcResult<()>;

    /// Check if hot-reload is supported
    fn supports_hot_reload(&self) -> bool;

    /// Get list of exported symbols
    fn exports(&self) -> Vec<String>;
}
```

The default implementation uses Wasmtime, but the trait allows alternative Wasm runtimes (WAMR, wasm3) or even native code for testing.

### FieldbusDriver Trait

The `FieldbusDriver` trait (`plc-fieldbus/src/lib.rs`) abstracts fieldbus communication:

```rust
pub trait FieldbusDriver: Send {
    /// Initialize the fieldbus connection
    fn init(&mut self) -> PlcResult<()>;

    /// Read inputs from field devices
    fn read_inputs(&mut self) -> PlcResult<FieldbusInputs>;

    /// Write outputs to field devices
    fn write_outputs(&mut self, outputs: &FieldbusOutputs) -> PlcResult<()>;

    /// Combined read/write for efficient fieldbus cycles
    fn exchange(&mut self) -> PlcResult<()>;

    /// Clean shutdown
    fn shutdown(&mut self) -> PlcResult<()>;
}
```

Implementations exist for EtherCAT (via SOEM), Modbus TCP, and simulated I/O.

### Scheduler

The `Scheduler<E: LogicEngine>` (`plc-runtime/src/scheduler.rs`) orchestrates the scan cycle:

```
┌─────────────────────────────────────────────────────┐
│                   Scan Cycle                        │
│                                                     │
│  1. Wait for cycle start ────────────────────────┐  │
│                                                  │  │
│  2. Read inputs from fieldbus ◄──────────────────┤  │
│     └─ Copy to process image                     │  │
│                                                  │  │
│  3. Execute user logic (Wasm) ◄──────────────────┤  │
│     └─ Read inputs from memory                   │  │
│     └─ Execute step() function                   │  │
│     └─ Write outputs to memory                   │  │
│                                                  │  │
│  4. Write outputs to fieldbus ◄──────────────────┤  │
│     └─ Copy from process image                   │  │
│                                                  │  │
│  5. Record metrics ◄─────────────────────────────┘  │
│     └─ Cycle time, overruns, jitter                 │
│                                                     │
└─────────────────────────────────────────────────────┘
```

### IoImage / Process Image

The `IoImage` (`plc-runtime/src/io_image.rs`) provides the shared memory interface between native code and Wasm:

```
Memory Layout (see process-image-abi.md for details):

Offset  Size   Description
0x00    4      Digital Inputs (32 bits)
0x04    4      Digital Outputs (32 bits)
0x08    32     Analog Inputs (16 x i16)
0x28    32     Analog Outputs (16 x i16)
0x48    32     System Info (see below)
0x68    ...    User Data Area

System Info Layout (32 bytes):
0x48    4      Cycle Time (u32 nanoseconds, capped at i32::MAX)
0x4C    4      Flags (bit 0 = first cycle, bit 1 = fault mode)
0x50    8      Cycle Count (u64)
0x58    4      Fault Code (u32, 0 = no fault)
0x5C    12     Reserved (zeroed)
```

The process image uses a double-buffering pattern for lock-free updates in the real-time path.

## WebAssembly Sandboxing

### Security Model

User logic runs in a Wasm sandbox with these restrictions:

| Capability | Allowed | Notes |
|------------|---------|-------|
| Memory access | Limited | Only process image region |
| System calls | No | No file, network, or OS access |
| Time access | Read-only | Via system info region |
| Infinite loops | Bounded | Fuel-limited execution |

### Memory Safety

The Wasm linear memory is isolated from the host:

```
┌─────────────────────────────────────────┐
│           Host Process Memory           │
│  ┌─────────────────────────────────┐   │
│  │        Wasm Linear Memory       │   │
│  │  ┌───────────────────────────┐  │   │
│  │  │    Process Image (R/W)    │  │   │
│  │  ├───────────────────────────┤  │   │
│  │  │    User Variables (R/W)   │  │   │
│  │  ├───────────────────────────┤  │   │
│  │  │    Stack (R/W)            │  │   │
│  │  └───────────────────────────┘  │   │
│  └─────────────────────────────────┘   │
│                                         │
│  Host data is INACCESSIBLE to Wasm     │
└─────────────────────────────────────────┘
```

### Hot-Reload

The Wasm sandbox enables safe hot-reload:

1. Validate new module has compatible interface
2. Optionally preserve linear memory (retains variable state)
3. Atomically swap at cycle boundary
4. Old module is dropped after swap completes

## Real-Time Considerations

### Linux PREEMPT_RT

For sub-millisecond determinism:

```bash
# Kernel configuration
CONFIG_PREEMPT_RT=y
CONFIG_NO_HZ_FULL=y
CONFIG_RCU_NOCB_CPU=y

# CPU isolation
isolcpus=2,3 nohz_full=2,3 rcu_nocbs=2,3

# Memory locking
mlockall(MCL_CURRENT | MCL_FUTURE)

# Real-time priority
sched_setscheduler(SCHED_FIFO, priority=90)
```

### Cycle Time Budget

For a 1ms cycle target:

```
Budget breakdown:
├── Fieldbus exchange:     200-400 µs (EtherCAT)
├── Wasm logic execution:  100-300 µs (typical)
├── Memory copies:         10-20 µs
├── Metrics recording:     5-10 µs
└── Margin:                270-685 µs
```

### Jitter Sources

| Source | Mitigation |
|--------|------------|
| OS scheduling | CPU isolation, SCHED_FIFO |
| Memory allocation | Pre-allocation, mlockall |
| Cache misses | Cache-aligned data structures |
| Interrupts | IRQ affinity to non-RT cores |

## Compiler Pipeline

```
Source (.st)
    │
    ▼
┌─────────┐
│  Lexer  │  Tokenization
└────┬────┘
     │
     ▼
┌─────────┐
│ Parser  │  AST construction (pest)
└────┬────┘
     │
     ▼
┌─────────┐
│Analyzer │  Type checking, symbol resolution
└────┬────┘
     │
     ▼
┌─────────┐
│   IR    │  Intermediate representation
└────┬────┘
     │
     ▼
┌─────────┐
│ Codegen │  Wasm emission
└────┬────┘
     │
     ▼
Output (.wasm)
```

## Monitoring Architecture

```
┌──────────────────────────────────────────────────────────┐
│                    Runtime Loop                          │
│                         │                                │
│              ┌──────────▼──────────┐                    │
│              │    StateUpdater     │                    │
│              └──────────┬──────────┘                    │
│                         │                                │
│         ┌───────────────┼───────────────┐               │
│         ▼               ▼               ▼               │
│   SharedState    Broadcast Channel  PlcMetrics          │
│         │               │               │               │
└─────────┼───────────────┼───────────────┼───────────────┘
          │               │               │
          ▼               ▼               ▼
    REST API (/api/*)  WebSocket (/ws)  Prometheus (/metrics)
```

### Metrics Collected

| Metric | Type | Description |
|--------|------|-------------|
| `plc_cycles_total` | Counter | Total scan cycles |
| `plc_cycle_time_*_us` | Gauge | Min/max/avg cycle time |
| `plc_overruns_total` | Counter | Cycle time exceeded |
| `plc_faults_total` | Counter | Fault conditions |
| `plc_runtime_state` | Gauge | Current state enum |
| `plc_digital_*` | Gauge | I/O state |
| `plc_analog_*` | GaugeVec | Per-channel analog values |

## State Machine

```
                    ┌──────────────┐
         ┌─────────│     Boot     │
         │         └──────┬───────┘
         │                │ initialize()
         │                ▼
         │         ┌──────────────┐
         │    ┌────│    PreOp     │────┐
         │    │    └──────┬───────┘    │
         │    │           │ start()    │ stop()
         │    │           ▼            │
         │    │    ┌──────────────┐    │
         │    │    │     Run      │────┤
         │    │    └──────┬───────┘    │
         │    │           │            │
         │    │           │ fault      │
         │    │           ▼            │
         │    │    ┌──────────────┐    │
         │    └───►│    Fault     │◄───┘
         │         └──────┬───────┘
         │                │ reset()
         └────────────────┘
```

## Further Reading

- [ADR-001: Split-Architecture Runtime](adr/001-runtime-arch.md)
- [ADR-002: WebAssembly Sandbox](adr/002-wasm-sandbox.md)
- [ADR-003: EtherCAT Primary Fieldbus](adr/003-ethercat-primary.md)
- [Process Image ABI](process-image-abi.md)
