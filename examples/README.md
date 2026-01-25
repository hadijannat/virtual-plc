# Virtual PLC Examples

This directory contains example IEC 61131-3 Structured Text programs demonstrating common industrial automation patterns.

## Examples

### blink.st - LED Blink

A simple introductory example that toggles a digital output at a configurable interval.

**Demonstrates:**
- Basic variable declarations with initialization
- Cycle-based timing (counting scan cycles)
- Boolean NOT operator for toggling
- IF/THEN/END_IF conditional logic

### motor_control.st - Motor Start/Stop with Interlocks

A typical industrial motor control pattern with safety features.

**Demonstrates:**
- Multiple input/output handling
- Safety interlock logic (E-stop, overload, guard sensors)
- Latching fault conditions
- Start/stop seal-in circuit pattern
- Restart delay with cycle counting
- NC (normally closed) contacts for fail-safe design

### state_machine.st - Batch Mixing Process

A sequential process control example simulating a batch mixing operation with multiple states.

**Demonstrates:**
- State machine pattern using CASE statement
- Sequential process control (FILL → MIX → HEAT → DRAIN)
- Cycle-based timing for operations
- Temperature control with hysteresis (bang-bang control)
- Emergency stop handling with safe state

### pid_control.st - PID Temperature Controller

A basic PID controller implementation for temperature regulation.

**Demonstrates:**
- Proportional-Integral-Derivative control algorithm
- Auto/Manual mode switching with bumpless transfer
- Output limiting and clamping
- Anti-windup for integral term (clamping + back-calculation)
- Derivative filtering to reduce noise
- Alarm handling

## Running Examples

To compile an example to WebAssembly:

```bash
# Compile to WebAssembly
cargo run -p plc-daemon -- compile examples/blink.st -o blink.wasm

# Validate the generated Wasm module
cargo run -p plc-daemon -- validate blink.wasm

# Simulate execution (dry run without hardware)
cargo run -p plc-daemon -- simulate blink.wasm --cycles 100

# Run with the daemon (requires fieldbus configuration)
cargo run -p plc-daemon -- run --wasm blink.wasm
```

## Scaling Conventions

Analog values in these examples use integer scaling for deterministic execution:

- **Temperature:** 0-10000 represents 0.0-100.0°C (0.01°C resolution)
- **Percentage:** 0-10000 represents 0.0-100.0% (0.01% resolution)
- **Timing:** Cycle-based counting (e.g., 50 cycles × 10ms = 500ms)

## Current Limitations

These examples use features currently supported by the compiler:
- `PROGRAM` blocks with `VAR` declarations
- Basic types: `BOOL`, `INT`, `UINT`, `REAL`, `STRING`
- Operators: `AND`, `OR`, `NOT`, `XOR`, arithmetic, comparisons
- Control flow: `IF`/`ELSIF`/`ELSE`, `CASE`, `FOR`, `WHILE`

**Not yet implemented:**
- `AT %` direct I/O addressing (variables are mapped via runtime config)
- `TON`/`TOF`/`TP` timer function blocks (use cycle counting instead)
- `FUNCTION_BLOCK` definitions
- `VAR_EXTERNAL` for global variables

See the compiler integration tests for additional feature examples.
