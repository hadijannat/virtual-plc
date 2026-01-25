# Getting Started with Virtual PLC

This guide walks you through setting up Virtual PLC and running your first program.

## Prerequisites

- **Rust toolchain**: 1.76 or later (`rustup update stable`)
- **Git**: For cloning the repository
- **Optional**: Docker for containerized deployment

## Installation

### Clone and Build

```bash
git clone https://github.com/your-org/virtual-plc.git
cd virtual-plc

# Build all crates
cargo build --release

# Verify the build
cargo run -p plc-daemon --release -- --help
```

### Verify Installation

```bash
# Run the test suite
cargo test

# Check the daemon starts correctly
cargo run -p plc-daemon -- --help
```

You should see output describing available commands and options.

## Your First Program

### 1. Write a Structured Text Program

Create a file called `counter.st`:

```iecst
PROGRAM Counter
VAR
    count : INT := 0;
    max_count : INT := 100;
    output_value : INT := 0;
END_VAR

count := count + 1;
IF count > max_count THEN
    count := 0;
END_IF;

output_value := count;

END_PROGRAM
```

This program increments a counter each scan cycle and resets at 100.

> **Note:** The variables (`output_value`) would be mapped to physical I/O via runtime configuration.
> Direct addressing (`AT %QW0`) is planned but not yet implemented.

### 2. Compile to WebAssembly

```bash
# Compile the ST program to WebAssembly
cargo run -p plc-daemon -- compile counter.st -o counter.wasm
```

The compiler translates IEC 61131-3 Structured Text into WebAssembly bytecode that runs in the sandboxed runtime.

### 3. Run in Simulation Mode

For initial testing without hardware:

```bash
# Run with simulated I/O
cargo run -p plc-daemon -- run --wasm counter.wasm --simulate
```

The `--simulate` flag creates virtual I/O that you can monitor through the web interface.

### 4. Monitor via Web UI

While the daemon is running, open your browser to:

```
http://localhost:8080
```

The web UI provides:
- Real-time I/O state visualization
- Cycle timing metrics
- Fault history
- Runtime state control

### 5. Access Prometheus Metrics

For integration with monitoring systems:

```
http://localhost:8080/metrics
```

Returns metrics in Prometheus text format, including cycle times, overrun counts, and I/O values.

## Project Structure

```
virtual-plc/
├── crates/
│   ├── plc-daemon/      # Main executable
│   ├── plc-runtime/     # Scheduler, Wasm host, I/O image
│   ├── plc-compiler/    # ST to Wasm compiler
│   ├── plc-fieldbus/    # EtherCAT, Modbus drivers
│   ├── plc-web-ui/      # REST API and WebSocket server
│   ├── plc-common/      # Shared types and errors
│   └── plc-stdlib/      # Standard function blocks
├── examples/            # Example ST programs
└── docs/                # Documentation
```

## Example Programs

The `examples/` directory contains ready-to-run programs:

| File | Description |
|------|-------------|
| `blink.st` | Simple LED blink using cycle counting |
| `motor_control.st` | Motor start/stop with safety interlocks |
| `state_machine.st` | Batch process sequential control (CASE statement) |
| `pid_control.st` | PID temperature controller with anti-windup |

Each example compiles successfully and demonstrates common industrial automation patterns. See `examples/README.md` for details on each program.

Run any example:

```bash
cargo run -p plc-daemon -- compile examples/blink.st -o blink.wasm
cargo run -p plc-daemon -- run --wasm blink.wasm --simulate
```

## Configuration

### Cycle Time

Configure the scan cycle period (default: 1ms):

```bash
cargo run -p plc-daemon -- run --wasm app.wasm --cycle-time 500us
```

### Web UI Port

Change the web server port:

```bash
cargo run -p plc-daemon -- run --wasm app.wasm --web-port 9090
```

### Fieldbus Configuration

For real hardware with EtherCAT:

```bash
cargo run -p plc-daemon -- run --wasm app.wasm \
    --fieldbus ethercat \
    --interface eth0
```

## Real-Time Setup (Linux)

For deterministic cycle times in production:

### 1. Install PREEMPT_RT Kernel

```bash
# Ubuntu/Debian
sudo apt install linux-image-rt-amd64
```

### 2. Configure CPU Isolation

Add to kernel parameters (`/etc/default/grub`):

```
GRUB_CMDLINE_LINUX="isolcpus=2,3 nohz_full=2,3 rcu_nocbs=2,3"
```

### 3. Run with Real-Time Priority

```bash
sudo chrt -f 90 cargo run -p plc-daemon --release -- run --wasm app.wasm
```

### 4. Verify Latency

```bash
# Run cyclictest to verify system latency
sudo cyclictest -p 90 -t1 -n -i 1000 -l 10000
```

Target: max latency < 50 microseconds for 1ms cycle time.

## Docker Deployment

```bash
# Build the container
docker compose build

# Run with real-time capabilities
docker compose up
```

The Docker configuration includes necessary capabilities for real-time operation (`SYS_NICE`, `IPC_LOCK`, `NET_RAW`).

## Troubleshooting

### "Permission denied" on network interface

EtherCAT requires raw socket access:

```bash
sudo setcap cap_net_raw+ep target/release/plc-daemon
```

### Cycle overruns

Check system load and consider:
- Isolating CPU cores for the PLC process
- Using PREEMPT_RT kernel
- Reducing cycle time requirements

### WebSocket connection fails

Ensure the web UI port isn't blocked by firewall:

```bash
sudo ufw allow 8080/tcp
```

## Next Steps

- Read the [Architecture Guide](architecture.md) for design details
- Explore [ADR-001](adr/001-runtime-arch.md) for split-plane rationale
- Review [ADR-002](adr/002-wasm-sandbox.md) for Wasm sandboxing benefits
- Check [Process Image ABI](process-image-abi.md) for I/O memory layout
