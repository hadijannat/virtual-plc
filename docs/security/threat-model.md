# Virtual PLC Security Threat Model

This document describes the security model of the Virtual PLC runtime, focusing on the WebAssembly sandbox that executes user-provided PLC logic.

## Security Objectives

1. **Isolation**: User logic cannot access host system resources
2. **Availability**: User logic cannot cause denial of service to the runtime
3. **Integrity**: User logic cannot corrupt runtime state or I/O data
4. **Confidentiality**: User logic cannot read host memory outside process image

## Trust Boundaries

```
┌─────────────────────────────────────────────────────────────┐
│                    TRUSTED ZONE                             │
│  ┌─────────────────────────────────────────────────────┐   │
│  │              Host Runtime (Rust)                    │   │
│  │  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐  │   │
│  │  │Scheduler│ │Fieldbus │ │ Web UI  │ │  Fault  │  │   │
│  │  │         │ │ Driver  │ │         │ │Recorder │  │   │
│  │  └────┬────┘ └─────────┘ └─────────┘ └─────────┘  │   │
│  │       │                                            │   │
│  │  ┌────▼────────────────────────────────────────┐  │   │
│  │  │           Wasmtime Runtime                  │  │   │
│  │  │  ┌──────────────────────────────────────┐  │  │   │
│  │  │  │         UNTRUSTED ZONE               │  │  │   │
│  │  │  │  ┌────────────────────────────────┐  │  │  │   │
│  │  │  │  │      User PLC Logic (Wasm)     │  │  │  │   │
│  │  │  │  │  ┌────────────┐ ┌──────────┐  │  │  │  │   │
│  │  │  │  │  │ User Code  │ │ Process  │  │  │  │  │   │
│  │  │  │  │  │            │ │  Image   │  │  │  │  │   │
│  │  │  │  │  └────────────┘ └──────────┘  │  │  │  │   │
│  │  │  │  └────────────────────────────────┘  │  │  │   │
│  │  │  └──────────────────────────────────────┘  │  │   │
│  │  └────────────────────────────────────────────┘  │   │
│  └─────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

## Threat Actors

### T1: Malicious Logic Author
- **Capability**: Can provide arbitrary Wasm bytecode
- **Goal**: Escape sandbox, access host resources, cause DoS
- **Mitigation**: Wasm sandbox, fuel limits, input validation

### T2: Compromised Upstream
- **Capability**: Can modify source code or dependencies
- **Goal**: Insert backdoors, weaken security
- **Mitigation**: Dependency auditing, reproducible builds, code review

### T3: Network Attacker
- **Capability**: Can send malicious data to fieldbus or web API
- **Goal**: Crash runtime, manipulate I/O, information disclosure
- **Mitigation**: Input validation, rate limiting, authentication

## Attack Surface Analysis

### AS1: Wasm Module Loading

**Entry Point**: `LogicEngine::load_wat()` / `LogicEngine::load_wasm()`

**Threats**:
| ID | Threat | Severity | Mitigation |
|----|--------|----------|------------|
| AS1.1 | Malformed Wasm crashes parser | High | Wasmtime validates before instantiation |
| AS1.2 | Wasm with excessive memory | Medium | Limit linear memory to 1MB |
| AS1.3 | Missing required exports | Low | Validate exports before accepting module |
| AS1.4 | Unexpected imports | High | Whitelist allowed imports |

**Validation Checklist**:
```rust
// Required exports
const REQUIRED_EXPORTS: &[&str] = &["init", "step", "memory"];

// Allowed imports (empty = no imports allowed)
const ALLOWED_IMPORTS: &[(&str, &str)] = &[
    ("env", "plc_trace"),
    ("env", "plc_fault"),
];
```

### AS2: Wasm Execution

**Entry Point**: `LogicEngine::step()`

**Threats**:
| ID | Threat | Severity | Mitigation |
|----|--------|----------|------------|
| AS2.1 | Infinite loop blocks cycle | Critical | Fuel-based execution limits |
| AS2.2 | Stack overflow | High | Wasmtime stack limits |
| AS2.3 | Out-of-bounds memory access | High | Wasm memory isolation |
| AS2.4 | Integer overflow in logic | Medium | User responsibility (documented) |

**Fuel Configuration**:
```rust
// Fuel budget calibration
// Typical: 1 fuel ≈ 1 Wasm instruction
// For 1ms cycle with 50% margin: 500,000 instructions @ 1 GIPS
const FUEL_BUDGET: u64 = 500_000;

// Fuel exhaustion handling
match store.consume_fuel(FUEL_BUDGET) {
    Ok(_) => { /* execution completed */ }
    Err(Trap::OutOfFuel) => {
        // Abort cycle, raise fault, enter safe state
        fault_recorder.record("Fuel exhausted - possible infinite loop");
        return Err(PlcError::Fault("Execution timeout".into()));
    }
}
```

### AS3: Process Image Interface

**Entry Point**: Shared memory region at Wasm linear memory base

**Threats**:
| ID | Threat | Severity | Mitigation |
|----|--------|----------|------------|
| AS3.1 | Write outside process image | High | Wasm memory bounds checking |
| AS3.2 | Invalid output values | Medium | Output range validation |
| AS3.3 | Time-of-check/time-of-use | Low | Double buffering, atomic copies |

**Memory Bounds**:
```
Process Image Layout (256 bytes):
┌──────────────────────────────────────────┐ 0x0000
│ Digital Inputs  (4 bytes, read-only*)    │
├──────────────────────────────────────────┤ 0x0004
│ Digital Outputs (4 bytes, read-write)    │
├──────────────────────────────────────────┤ 0x0008
│ Analog Inputs   (32 bytes, read-only*)   │
├──────────────────────────────────────────┤ 0x0028
│ Analog Outputs  (32 bytes, read-write)   │
├──────────────────────────────────────────┤ 0x0048
│ System Info     (32 bytes, read-only*)   │
└──────────────────────────────────────────┘ 0x0068

* "read-only" enforced by convention; Wasm can write but
  values are overwritten by host before each cycle
```

### AS4: Host Function Interface

**Entry Point**: Imported functions `plc_trace`, `plc_fault`

**Threats**:
| ID | Threat | Severity | Mitigation |
|----|--------|----------|------------|
| AS4.1 | Excessive trace calls (DoS) | Medium | Rate limiting |
| AS4.2 | Trace buffer overflow | High | Fixed-size ring buffer |
| AS4.3 | Malicious fault triggering | Low | Intentional capability |

**Host Function Implementation**:
```rust
// Rate-limited trace function
fn plc_trace(caller: Caller<'_, HostState>, ptr: i32, len: i32) {
    let state = caller.data();

    // Rate limit: max 100 traces per cycle
    if state.trace_count >= 100 {
        return;
    }
    state.trace_count += 1;

    // Validate bounds
    let memory = caller.get_export("memory")?.into_memory()?;
    if ptr < 0 || len < 0 || len > 256 {
        return;
    }

    // Safe read from Wasm memory
    let data = memory.data(&caller);
    let slice = &data[ptr as usize..(ptr + len) as usize];

    // Log to ring buffer (doesn't allocate)
    state.trace_buffer.push(slice);
}
```

### AS5: Hot-Reload

**Entry Point**: `LogicEngine::reload_module()`

**Threats**:
| ID | Threat | Severity | Mitigation |
|----|--------|----------|------------|
| AS5.1 | Incompatible module swap | High | Interface validation |
| AS5.2 | State corruption on reload | Medium | Atomic swap at cycle boundary |
| AS5.3 | Memory preservation attack | Low | Clear sensitive regions |

**Safe Reload Protocol**:
1. Validate new module has required exports
2. Wait for current cycle to complete
3. Optionally preserve user variable region
4. Clear process image outputs (fail-safe)
5. Instantiate new module
6. Call `init()` on new instance
7. Resume normal operation

### AS6: Web API / WebSocket

**Entry Point**: HTTP/WebSocket on port 8080

**Threats**:
| ID | Threat | Severity | Mitigation |
|----|--------|----------|------------|
| AS6.1 | Unauthorized state access | Medium | Authentication (TODO) |
| AS6.2 | WebSocket flood | Medium | Connection limits, rate limiting |
| AS6.3 | XSS via fault messages | Low | HTML escaping |
| AS6.4 | CORS bypass | Low | Configurable CORS policy |

**Current Mitigations**:
- WebSocket broadcast uses bounded channel (256 slots)
- Lagging clients are dropped, not buffered
- No write operations exposed via API (read-only)

**Future Improvements**:
- Add API authentication (JWT or API keys)
- Rate limit REST endpoints
- Add TLS support

## Security Controls Summary

| Control | Status | Description |
|---------|--------|-------------|
| Memory Isolation | ✅ Implemented | Wasm linear memory sandbox |
| Execution Limits | ✅ Implemented | Fuel-based timeout |
| Input Validation | ✅ Implemented | Module validation on load |
| Host Function Safety | ✅ Implemented | Rate limiting, bounds checks |
| API Authentication | ❌ Not Implemented | Planned for future release |
| TLS | ❌ Not Implemented | Planned for future release |
| Audit Logging | ⚠️ Partial | Fault recording only |

## Residual Risks

| Risk | Likelihood | Impact | Acceptance |
|------|------------|--------|------------|
| Wasmtime vulnerability | Low | Critical | Monitor CVEs, update promptly |
| Side-channel attacks | Low | Medium | Accept for non-crypto workloads |
| Fieldbus injection | Medium | High | Defense in depth at network layer |
| Physical access | N/A | Critical | Out of scope (physical security) |

## Security Testing

### Recommended Tests

1. **Fuzz Testing**: Fuzz Wasm module loading with malformed input
2. **Boundary Testing**: Test process image bounds enforcement
3. **Fuel Testing**: Verify infinite loop detection
4. **Resource Testing**: Test memory limits under load

### Fuzzing Targets

```bash
# Install cargo-fuzz
cargo install cargo-fuzz

# Fuzz the ST parser
cd crates/plc-compiler
cargo fuzz run fuzz_parse

# Fuzz Wasm module loading
cd crates/plc-runtime
cargo fuzz run fuzz_load_wasm
```

## Incident Response

### Suspected Sandbox Escape

1. Immediately stop all PLC instances
2. Preserve logs and core dumps
3. Isolate affected system from network
4. Report to security team
5. Do not resume until root cause identified

### Denial of Service

1. Check fuel exhaustion in fault log
2. Review recent module changes
3. Consider reducing fuel budget
4. Implement additional rate limiting if web-based

## References

- [WebAssembly Security Model](https://webassembly.org/docs/security/)
- [Wasmtime Security Documentation](https://docs.wasmtime.dev/security.html)
- [Bytecode Alliance Security Policy](https://bytecodealliance.org/security)
- [IEC 62443 Industrial Cybersecurity](https://www.isa.org/standards-and-publications/isa-standards/isa-iec-62443-series-of-standards)
