# Changelog

All notable changes to Virtual PLC will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

#### Core Runtime
- **WebAssembly Sandboxing**: User PLC logic runs in isolated Wasm sandbox using Wasmtime
- **Split-Plane Architecture**: Fieldbus I/O decoupled from logic execution for fault isolation
- **Cyclic Scheduler**: Deterministic scan loop with configurable cycle time
- **Fault Recorder**: Pre-fault buffer captures I/O state for post-mortem analysis
- **Watchdog Timer**: Configurable watchdog with cycle overrun detection
- **Real-Time Support**: PREEMPT_RT kernel support with CPU affinity and memory locking

#### Compiler
- **IEC 61131-3 ST Compiler**: Structured Text to WebAssembly compilation
- **Full Language Support**: PROGRAM, FUNCTION, VAR blocks, control flow (IF/CASE/FOR/WHILE)
- **Data Types**: BOOL, INT, UINT, DINT, REAL, LREAL, STRING, TIME
- **Operators**: Arithmetic, comparison, boolean (AND, OR, XOR, NOT)
- **Fuzzing Infrastructure**: Parser and compiler fuzz targets with seed corpus

#### CLI (plc-daemon)
- **`run` subcommand**: Start PLC runtime with Wasm module
- **`compile` subcommand**: Compile ST source to WebAssembly
- **`validate` subcommand**: Validate Wasm module structure and exports
- **`simulate` subcommand**: Run Wasm module without fieldbus hardware
- **`diagnose` subcommand**: System capability assessment with JSON output
  - PREEMPT_RT kernel detection
  - CPU isolation configuration check
  - Memory locking capability test
  - Network interface listing with EtherCAT suitability flags
  - Timing jitter measurement

#### Fieldbus Support
- **EtherCAT**: Framework with distributed clock support (SOEM integration scaffolded)
- **Modbus TCP**: Full implementation with all standard function codes
  - Read Coils (0x01), Read Holding Registers (0x03)
  - Write Single Coil (0x05), Write Multiple Registers (0x10)
  - Connection pooling and reconnection logic
  - Exception handling

#### Web UI
- **REST API**: Endpoints for state, metrics, I/O, and faults
- **WebSocket Streaming**: Real-time I/O state updates
- **Embedded Dashboard**: No-build HTML/CSS/JS dashboard
  - Digital I/O visualization (32 bits)
  - Analog I/O bar graphs (4 channels)
  - Cycle metrics display
  - Fault history
- **Prometheus Metrics**: `/metrics` endpoint for monitoring integration

#### Hot-Reload
- **SIGHUP Handler**: Reload Wasm module without stopping I/O plane
- **Memory Preservation**: Option to preserve user variable state across reload
- **Atomic Swap**: Module replacement at cycle boundary

#### Developer Experience
- **VS Code Extension**: Structured Text syntax highlighting
  - TextMate grammar for keywords, types, operators
  - Code folding and bracket matching
  - Time literal and I/O address highlighting
- **Example Programs**: Four working examples in `examples/`
  - `blink.st` - LED blink with cycle counting
  - `motor_control.st` - Safety interlock pattern
  - `state_machine.st` - Batch process control
  - `pid_control.st` - Temperature controller with anti-windup

#### Documentation
- **Architecture Decision Records**: ADR-001 (Split Plane), ADR-002 (Wasm Sandbox), ADR-003 (EtherCAT)
- **Getting Started Guide**: First program to running
- **Security Threat Model**: Attack surface analysis and mitigations
- **Process Image ABI**: Memory layout specification

#### CI/CD
- **GitHub Actions**: Multi-platform testing (Linux, macOS, Windows)
- **Docker Support**: Container build with real-time capabilities
- **Release Artifacts**: Automated binary builds

### Changed
- Improved error handling throughout runtime (removed unwrap() in production paths)
- Enhanced diagnose command with comprehensive system checks

### Security
- Wasm fuel-based execution limits prevent infinite loops
- Process image bounds checking
- Rate-limited host functions (plc_trace)
- WebSocket broadcast channel prevents memory exhaustion

## [0.0.1] - 2024-01-15

### Added
- Initial project structure
- Basic Wasm host implementation
- Scheduler framework
- Preliminary fieldbus abstractions

---

[Unreleased]: https://github.com/your-org/virtual-plc/compare/v0.0.1...HEAD
[0.0.1]: https://github.com/your-org/virtual-plc/releases/tag/v0.0.1
