# ADR-001: Split-Architecture Runtime

**Status:** Accepted

**Date:** 2024-01-15

## Context

Industrial PLC systems require:
- **Deterministic execution**: Cycle times must be predictable and bounded
- **Fault tolerance**: Logic errors should not crash the entire system
- **Updatability**: Logic should be updatable without stopping I/O
- **Security**: User-provided logic should not access arbitrary system resources

Traditional soft PLCs run user logic in the same process as I/O handling, creating tight coupling that makes these goals difficult to achieve.

### Problem Statement

How do we structure the runtime to achieve fault isolation between user logic and I/O handling while maintaining deterministic real-time performance?

### Constraints

1. Cycle times must be achievable in sub-millisecond range (target: 250µs - 1ms)
2. User logic bugs must not crash fieldbus communication
3. Logic updates should be possible without stopping I/O
4. The system must run on standard Linux with PREEMPT_RT

### Options Considered

**Option A: Monolithic Process**
- Single process with all components
- Pros: Simple, low latency between components
- Cons: No fault isolation, single point of failure

**Option B: Separate Processes with IPC**
- Logic and I/O in separate OS processes
- Pros: Strong isolation via OS boundaries
- Cons: IPC overhead (10-100µs), complex synchronization

**Option C: In-Process Isolation with Wasm Sandbox**
- Single process, logic runs in Wasm sandbox
- Pros: Low overhead (<1µs), memory isolation, portable logic
- Cons: Requires Wasm runtime, limited to Wasm capabilities

## Decision

We adopt **Option C: In-Process Isolation with Wasm Sandbox**, organizing the runtime into three logical planes:

1. **Fieldbus Plane**: Handles all I/O communication (EtherCAT, Modbus)
2. **Logic Plane**: Executes user logic in Wasm sandbox
3. **Control Plane**: Provides monitoring, configuration, and diagnostics

The planes share a common **Process Image** - a defined memory region for I/O exchange.

### Architecture Diagram

```
┌─────────────────────────────────────────────┐
│              Control Plane                  │
│  (REST API, WebSocket, Prometheus)          │
└─────────────────────┬───────────────────────┘
                      │
┌─────────────────────┼───────────────────────┐
│              Logic Plane                    │
│  ┌─────────────┐    │    ┌──────────────┐  │
│  │    Wasm     │◄───┼───►│   Process    │  │
│  │   Sandbox   │         │    Image     │  │
│  └─────────────┘         └──────┬───────┘  │
└─────────────────────────────────┼──────────┘
                                  │
┌─────────────────────────────────┼──────────┐
│              Fieldbus Plane                │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐    │
│  │EtherCAT │  │ Modbus  │  │  Sim    │    │
│  └─────────┘  └─────────┘  └─────────┘    │
└────────────────────────────────────────────┘
```

## Consequences

### Positive

1. **Fault Isolation**: Wasm sandbox prevents user logic from corrupting I/O state
2. **Hot-Reload**: Logic can be swapped at cycle boundaries without stopping I/O
3. **Portability**: Same Wasm module runs on any platform with a Wasm runtime
4. **Security**: User logic cannot access filesystem, network, or system calls
5. **Testability**: Each plane can be unit tested independently
6. **Low Overhead**: Wasm function calls add <1µs compared to native

### Negative

1. **Wasm Limitations**: No direct hardware access, limited floating-point precision
2. **Debugging Complexity**: Wasm debugging tools less mature than native
3. **Memory Overhead**: Wasm linear memory adds ~10-100KB per instance
4. **Compilation Step**: User code must be compiled to Wasm before deployment

### Neutral

1. Requires maintaining the ST-to-Wasm compiler
2. Process image ABI must be carefully versioned
3. Real-time guarantees still depend on proper OS configuration

## Implementation Notes

- The `Scheduler` component coordinates the scan cycle across planes
- Process Image uses double-buffering for lock-free I/O exchange
- Wasm fuel limits prevent infinite loops from blocking the cycle
- Control plane updates are queued and processed asynchronously

## References

- [WebAssembly Core Specification](https://webassembly.github.io/spec/core/)
- [IEC 61131-3 Programming Languages](https://www.plcopen.org/iec-61131-3)
- [EtherCAT Technology Group](https://www.ethercat.org/)
