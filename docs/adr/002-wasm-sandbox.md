# ADR-002: WebAssembly Sandbox for User Logic

**Status:** Accepted

**Date:** 2024-01-15

**Supersedes:** None

**Related:** [ADR-001](001-runtime-arch.md) (Split-Architecture Runtime)

## Context

Per ADR-001, we need an isolation mechanism for user logic that provides:
- Memory safety (logic cannot corrupt runtime state)
- Execution bounds (infinite loops must be detectable)
- Portability (same logic runs on different platforms)
- Low overhead (<10µs per cycle)

### Problem Statement

What technology should we use to sandbox user logic execution while maintaining real-time performance characteristics?

### Constraints

1. Must support deterministic execution (no GC pauses, predictable timing)
2. Must provide memory isolation between user code and host
3. Must allow efficient I/O through shared memory
4. Should support multiple source languages (ST, potentially others)
5. Must have mature, production-ready implementations

### Options Considered

**Option A: Native Code with Process Isolation**
- Compile ST to native code, run in separate process
- Pros: Fastest execution, strong OS-level isolation
- Cons: IPC overhead (10-100µs), platform-specific compilation

**Option B: eBPF**
- Compile ST to eBPF bytecode
- Pros: Kernel-level verification, very fast
- Cons: Limited instruction set, Linux-only, not designed for control logic

**Option C: Lua/LuaJIT**
- Embed Lua interpreter
- Pros: Simple integration, fast JIT
- Cons: GC pauses, no memory isolation, dynamic typing

**Option D: WebAssembly**
- Compile ST to Wasm, run in Wasm runtime (Wasmtime/WAMR)
- Pros: Memory isolation, portable, deterministic, fuel-based limits
- Cons: Slightly slower than native, requires compilation step

**Option E: Custom Bytecode VM**
- Design PLC-specific bytecode format
- Pros: Optimized for control logic, full control
- Cons: Massive development effort, no ecosystem

## Decision

We adopt **Option D: WebAssembly** with **Wasmtime** as the primary runtime.

### Rationale

1. **Memory Safety**: Wasm linear memory is isolated from host memory by design
2. **Fuel Metering**: Wasmtime's fuel system allows bounding execution time
3. **Portability**: Same .wasm file runs on Linux, Windows, embedded
4. **Ecosystem**: Growing toolchain support, WASI standardization
5. **Performance**: AOT compilation achieves near-native speed
6. **Hot-Reload**: Module replacement is a first-class operation

### Runtime Selection

| Runtime | Pros | Cons | Use Case |
|---------|------|------|----------|
| Wasmtime | Fast AOT, fuel metering, mature | Larger binary, slower startup | Primary runtime |
| WAMR | Small footprint, embedded-friendly | Less mature fuel support | Embedded targets |
| wasm3 | Tiny, interpreter | Slower execution | Resource-constrained |

We use Wasmtime as the default with the `LogicEngine` trait allowing alternative implementations.

## Implementation Details

### Memory Layout

```
Wasm Linear Memory (min 1 page = 64KB):
┌────────────────────────────────────────┐ 0x0000
│           Process Image (256B)         │
│  ├─ Digital Inputs     [0x00-0x04)     │
│  ├─ Digital Outputs    [0x04-0x08)     │
│  ├─ Analog Inputs      [0x08-0x28)     │
│  ├─ Analog Outputs     [0x28-0x48)     │
│  ├─ Cycle Time         [0x48-0x50)     │
│  └─ System Flags       [0x50-0x54)     │
├────────────────────────────────────────┤ 0x0100
│           User Variables               │
│           (compiler-allocated)         │
├────────────────────────────────────────┤
│           Stack                        │
│           (grows downward)             │
└────────────────────────────────────────┘ 0xFFFF
```

### Fuel Budget

Fuel is consumed per Wasm instruction. We configure:

```rust
// Approximately 1 fuel per simple instruction
// Budget for 1ms cycle: ~1,000,000 fuel (assumes 1 GIPS)
const FUEL_PER_CYCLE: u64 = 1_000_000;
```

If fuel is exhausted, the cycle is aborted and a fault is raised.

### Host Functions

Minimal host function interface:

```wat
;; Imported from host
(import "env" "plc_trace" (func $trace (param i32 i32)))
(import "env" "plc_fault" (func $fault (param i32)))

;; Exported to host
(export "init" (func $init))
(export "step" (func $step))
(export "fault" (func $fault_handler))
(export "memory" (memory $mem))
```

### Security Boundaries

| Capability | Wasm Guest | Host |
|------------|------------|------|
| Read process image | Yes | Yes |
| Write process image | Yes | Yes |
| Access host memory | No | Yes |
| System calls | No | Yes |
| File access | No | Yes |
| Network access | No | Yes |
| Time (real) | No | Yes |
| Time (cycle) | Read-only | Yes |

## Consequences

### Positive

1. **Strong Isolation**: Memory corruption in user logic cannot affect host
2. **Deterministic Execution**: No GC, predictable instruction timing
3. **Fuel Limits**: Infinite loops detected and aborted
4. **Hot-Reload**: New module can be loaded without process restart
5. **Cross-Platform**: Same .wasm runs on any supported OS/architecture
6. **Future-Proof**: Wasm ecosystem growing rapidly

### Negative

1. **Compilation Required**: ST must be compiled to Wasm before deployment
2. **Debugging**: Source-level debugging requires DWARF support in toolchain
3. **Floating Point**: Wasm f32/f64 may differ slightly from native IEEE 754
4. **No Direct I/O**: All I/O must go through process image (no direct port access)

### Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Wasm runtime bug exposes host | Use well-audited runtime (Wasmtime), keep updated |
| Fuel calibration incorrect | Measure actual execution, add safety margin |
| Memory exhaustion | Pre-allocate, limit linear memory growth |
| Performance regression | Benchmark on each release, profile hot paths |

## Verification

To verify isolation:

1. **Fuzzing**: Fuzz the Wasm module loading path
2. **Boundary Tests**: Test memory access at process image boundaries
3. **Fuel Tests**: Verify infinite loops are caught
4. **Integration Tests**: Run malformed modules, verify graceful failure

## References

- [WebAssembly Specification](https://webassembly.github.io/spec/)
- [Wasmtime Documentation](https://docs.wasmtime.dev/)
- [Bytecode Alliance Security Practices](https://bytecodealliance.org/)
- [WAMR (WebAssembly Micro Runtime)](https://github.com/bytecodealliance/wasm-micro-runtime)
