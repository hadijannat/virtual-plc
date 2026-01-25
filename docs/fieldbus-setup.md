# Fieldbus Setup Guide

This guide covers hardware requirements, system configuration, and troubleshooting for Virtual PLC fieldbus communication. The runtime supports two primary fieldbus protocols: Modbus TCP for legacy and simple deployments, and EtherCAT for high-performance, deterministic control.

## Table of Contents

- [Protocol Comparison](#protocol-comparison)
- [Modbus TCP](#modbus-tcp)
  - [Server Requirements](#server-requirements)
  - [Configuration](#modbus-tcp-configuration)
  - [Testing with Simulators](#testing-with-simulators)
  - [I/O Mapping](#modbus-io-mapping)
  - [Troubleshooting](#modbus-troubleshooting)
- [EtherCAT](#ethercat)
  - [Hardware Requirements](#hardware-requirements)
  - [NIC Recommendations](#nic-recommendations)
  - [System Configuration](#system-configuration)
  - [Configuration](#ethercat-configuration)
  - [Distributed Clocks](#distributed-clocks)
  - [Expected Log Output](#expected-log-output)
  - [Troubleshooting](#ethercat-troubleshooting)
- [Simulated I/O](#simulated-io)
- [Real-Time Linux Setup](#real-time-linux-setup)

---

## Protocol Comparison

| Feature | Modbus TCP | EtherCAT |
|---------|------------|----------|
| Min Cycle Time | ~10 ms | < 100 us |
| Determinism | Non-deterministic | Hardware-level |
| Max Devices | 247 (per network) | 65535 |
| Topology | Star | Line/Tree |
| Hardware | Standard Ethernet | Dedicated NIC |
| Complexity | Simple | Complex |
| Use Case | SCADA, HMI, legacy | Motion control, precision timing |

Choose **Modbus TCP** for:
- Integration with legacy equipment
- Non-time-critical I/O
- PLC-to-PLC data exchange
- Building automation, HVAC

Choose **EtherCAT** for:
- Motion control applications
- Sub-millisecond cycle times
- Coordinated multi-axis movements
- Precision timing across devices

---

## Modbus TCP

Modbus TCP is a widely supported protocol that runs over standard Ethernet. Virtual PLC implements a full Modbus TCP client with support for reading and writing coils, discrete inputs, and registers.

### Server Requirements

- Standard Modbus TCP port: **502** (or custom)
- TCP keepalive recommended for connection health
- Server must respond within configured timeout (default: 1 second)
- Supported devices include:
  - Industrial PLCs (Siemens, Allen-Bradley, Schneider)
  - Remote I/O modules
  - Building automation controllers
  - Soft Modbus servers for testing

### Modbus TCP Configuration

Create a configuration file (e.g., `config/modbus.toml`):

```toml
# Runtime settings
cycle_time = "10ms"

[metrics]
http_export = true
http_port = 8080

[fieldbus]
driver = "modbustcp"

[fieldbus.modbus]
# Server address (IP:port)
# Standard Modbus TCP port is 502
address = "192.168.1.100:502"

# Unit identifier (slave ID)
# Typically 1 for direct TCP connections
# May vary for Modbus gateways (1-247)
unit_id = 1

# Connection and response timeout
timeout = "1s"

# I/O Mapping
# Digital input coils (read-only from PLC perspective)
digital_input_coils = { address = 0, quantity = 32 }

# Digital output coils (read-write)
digital_output_coils = { address = 100, quantity = 32 }

# Analog input registers (16-bit)
analog_input_registers = { address = 0, quantity = 8 }

# Analog output registers (16-bit holding registers)
analog_output_registers = { address = 100, quantity = 8 }
```

Run the daemon with the configuration:

```bash
cargo run -p plc-daemon -- run \
    --config config/modbus.toml \
    --wasm-module app.wasm
```

### Testing with Simulators

For development and testing without physical hardware, use a Modbus TCP simulator.

#### Using pymodbus

Install and run a Modbus TCP simulator:

```bash
pip install pymodbus

# Start simulator on port 5020
pymodbus.simulator --host 127.0.0.1 --port 5020
```

Update your configuration to point to the simulator:

```toml
[fieldbus.modbus]
address = "127.0.0.1:5020"
unit_id = 1
timeout = "1s"
```

#### Using diagslave (Windows/Linux)

Download diagslave from the Modbus Tools website and run:

```bash
# Linux
./diagslave -m tcp -p 502

# Windows
diagslave.exe -m tcp -p 502
```

### Modbus I/O Mapping

The Modbus driver maps I/O to the Virtual PLC process image:

| Process Image | Modbus Function | Default Address |
|---------------|-----------------|-----------------|
| Digital Inputs (32 bits) | FC 0x02 (Read Discrete Inputs) | 0-31 |
| Digital Outputs (32 bits) | FC 0x0F (Write Multiple Coils) | 100-131 |
| Analog Inputs (16 x i16) | FC 0x04 (Read Input Registers) | 0-15 |
| Analog Outputs (16 x i16) | FC 0x10 (Write Multiple Registers) | 100-115 |

#### Supported Function Codes

| Code | Function | Direction |
|------|----------|-----------|
| 0x01 | Read Coils | Read |
| 0x02 | Read Discrete Inputs | Read |
| 0x03 | Read Holding Registers | Read |
| 0x04 | Read Input Registers | Read |
| 0x05 | Write Single Coil | Write |
| 0x06 | Write Single Register | Write |
| 0x0F | Write Multiple Coils | Write |
| 0x10 | Write Multiple Registers | Write |

### Modbus Troubleshooting

#### Connection Refused

```
Error: Connection failed: Connection refused (os error 111)
```

**Causes and solutions:**

1. **Server not running**: Verify the Modbus server is active
   ```bash
   # Check if port is listening
   netstat -tlnp | grep 502
   ```

2. **Firewall blocking**: Allow traffic on the Modbus port
   ```bash
   sudo ufw allow 502/tcp
   ```

3. **Wrong address**: Verify IP and port in configuration

#### Timeout Errors

```
Error: Receive header failed: timed out
```

**Solutions:**

1. Increase timeout in configuration:
   ```toml
   [fieldbus.modbus]
   timeout = "5s"
   ```

2. Check network latency:
   ```bash
   ping 192.168.1.100
   ```

3. Reduce cycle time to allow more time for communication

#### Exception Responses

Modbus exceptions indicate protocol-level errors from the server:

| Code | Exception | Cause | Solution |
|------|-----------|-------|----------|
| 0x01 | Illegal Function | Function not supported | Check device documentation |
| 0x02 | Illegal Data Address | Register address out of range | Verify address mapping |
| 0x03 | Illegal Data Value | Value outside acceptable range | Check data constraints |
| 0x04 | Server Device Failure | Internal device error | Check device status |
| 0x06 | Server Device Busy | Device processing another request | Add retry delay |
| 0x0A | Gateway Path Unavailable | Gateway cannot reach target | Check gateway configuration |
| 0x0B | Gateway Target Failed | Target device not responding | Check target device |

#### Reconnection Behavior

The Modbus driver implements automatic reconnection with configurable parameters:

- **Max attempts**: 3 (default)
- **Retry delay**: 500 ms (default)
- **Non-blocking**: Reconnection attempts do not block the PLC cycle

If the server becomes unavailable, the driver will attempt to reconnect. After exceeding the maximum attempts, a fault is triggered.

---

## EtherCAT

EtherCAT provides deterministic, sub-millisecond communication for high-performance industrial control. Virtual PLC uses SOEM (Simple Open EtherCAT Master) as the underlying master stack.

### Hardware Requirements

- **Dedicated network interface**: EtherCAT traffic should not share a NIC with other network traffic
- **EtherCAT slaves**: I/O modules, drives, or sensors with EtherCAT interface
- **Cables**: Standard CAT5e/CAT6 Ethernet cables

### NIC Recommendations

EtherCAT performance heavily depends on the network interface controller.

#### Recommended NICs

| NIC | Ports | Notes |
|-----|-------|-------|
| Intel i210 | 1 | Excellent timing, widely available |
| Intel i350 | 2-4 | Dual/quad port server adapter |
| Intel i225 | 1 | 2.5 GbE, newer systems |
| Intel I219-V | 1 | Onboard, good performance |

These NICs provide consistent, low-latency frame handling suitable for real-time EtherCAT.

#### Avoid

| NIC Type | Issue |
|----------|-------|
| Realtek consumer NICs | Inconsistent timing, high jitter |
| USB Ethernet adapters | Additional latency, no raw socket control |
| Virtual machine passthrough | Unless using SR-IOV with dedicated hardware |
| WiFi adapters | Non-deterministic, not suitable for EtherCAT |

### System Configuration

EtherCAT requires raw socket access and system-level configuration for optimal performance.

#### 1. Identify Network Interface

```bash
# List network interfaces
ip link show

# Example output:
# 1: lo: <LOOPBACK,UP,LOWER_UP>
# 2: enp3s0: <BROADCAST,MULTICAST,UP,LOWER_UP>
# 3: enp4s0: <BROADCAST,MULTICAST>

# Use a dedicated interface (e.g., enp3s0)
```

#### 2. Set Interface to Promiscuous Mode

```bash
# Enable promiscuous mode (required for EtherCAT)
sudo ip link set enp3s0 promisc on

# Verify
ip link show enp3s0 | grep PROMISC
```

#### 3. Grant Raw Socket Capability

Option A: Run as root (not recommended for production):
```bash
sudo ./target/release/plc-daemon run --config config/ethercat.toml
```

Option B: Grant capability to binary (recommended):
```bash
sudo setcap cap_net_raw+ep ./target/release/plc-daemon

# Verify capability
getcap ./target/release/plc-daemon
# Expected: ./target/release/plc-daemon cap_net_raw=ep
```

#### 4. Disable Network Manager (Optional)

Prevent Network Manager from interfering with the EtherCAT interface:

```bash
# Create configuration file
sudo tee /etc/NetworkManager/conf.d/unmanaged-ethercat.conf << EOF
[keyfile]
unmanaged-devices=interface-name:enp3s0
EOF

# Restart Network Manager
sudo systemctl restart NetworkManager
```

### EtherCAT Configuration

Create a configuration file (e.g., `config/ethercat.toml`):

```toml
# Runtime settings
cycle_time = "1ms"

[metrics]
http_export = true
http_port = 8080

[fieldbus]
driver = "ethercat"

[fieldbus.ethercat]
# Network interface (use `ip link` to find)
interface = "enp3s0"

# Distributed Clocks configuration
dc_enabled = true
dc_sync0_cycle = "1ms"

# Working counter error threshold
# Fault triggered after N consecutive WKC errors
# Set to 0 to disable (only log warnings)
wkc_error_threshold = 3
```

Run the daemon:

```bash
cargo run -p plc-daemon --release --features soem -- run \
    --config config/ethercat.toml \
    --wasm-module app.wasm
```

### Distributed Clocks

Distributed Clocks (DC) synchronize outputs across all EtherCAT slaves with sub-microsecond precision. This is essential for:

- Motion control (coordinated axis movements)
- Precise output timing
- Synchronized data acquisition

#### DC Configuration Parameters

| Parameter | Description | Typical Values |
|-----------|-------------|----------------|
| `dc_enabled` | Enable DC synchronization | `true` for motion, `false` for simple I/O |
| `dc_sync0_cycle` | Sync0 event cycle time | Should match or divide `cycle_time` |

#### DC Sync Modes

The EtherCAT master automatically configures DC mode based on slave capabilities:

| Mode | Description |
|------|-------------|
| Free-run | No synchronization (default if DC disabled) |
| DC Sync0 | Outputs synchronized to Sync0 event |
| DC Sync0+Sync1 | Two sync events (complex applications) |

### Expected Log Output

A successful EtherCAT startup produces log output similar to:

```
2024-01-15T10:00:00.123Z  INFO plc_fieldbus::ethercat: Scanning for EtherCAT slaves interface="enp3s0"
2024-01-15T10:00:00.234Z  INFO plc_fieldbus::ethercat: Found EtherCAT slaves slave_count=3
2024-01-15T10:00:00.235Z DEBUG plc_fieldbus::ethercat: Discovered slave position=0 name="EL1008" vendor=0x00000002 product=0x03F03052 dc=true
2024-01-15T10:00:00.236Z DEBUG plc_fieldbus::ethercat: Discovered slave position=1 name="EL2008" vendor=0x00000002 product=0x07D83052 dc=true
2024-01-15T10:00:00.237Z DEBUG plc_fieldbus::ethercat: Discovered slave position=2 name="EL3102" vendor=0x00000002 product=0x0C1E3052 dc=true
2024-01-15T10:00:00.300Z  INFO plc_fieldbus::ethercat: Slave scan complete slave_count=3 input_size=6 output_size=2 expected_wkc=6
2024-01-15T10:00:00.301Z  INFO plc_fieldbus::ethercat: All slaves configured to PRE_OP
2024-01-15T10:00:00.350Z  INFO plc_fieldbus::dc_sync: DC synchronization initialized reference_clock=0 cycle_time_ns=1000000 slave_count=3
2024-01-15T10:00:00.400Z  INFO plc_fieldbus::ethercat: All slaves in SAFE_OP
2024-01-15T10:00:00.450Z  INFO plc_fieldbus::ethercat: All slaves in OP - cyclic exchange active
2024-01-15T10:00:00.451Z  INFO plc_runtime::scheduler: PLC runtime started cycle_time="1ms"
```

#### State Transitions

The EtherCAT master transitions through these states:

```
OFFLINE -> INIT -> PRE_OP -> SAFE_OP -> OP
    |                                    |
    +<---- (shutdown or fault) <---------+
```

| State | Description |
|-------|-------------|
| OFFLINE | Master not initialized |
| INIT | Scanning for slaves |
| PRE_OP | Slaves discovered, configuring PDO mapping |
| SAFE_OP | DC configured, outputs in safe state |
| OP | Full operation, cyclic exchange active |
| FAULT | Error state, requires reset |

### EtherCAT Troubleshooting

#### No Slaves Found

```
WARN plc_fieldbus::ethercat: No EtherCAT slaves found on the network
```

**Causes and solutions:**

1. **Wrong interface name**: Verify with `ip link show`

2. **Interface not up**: Bring up the interface
   ```bash
   sudo ip link set enp3s0 up
   ```

3. **Cable disconnected**: Check physical connections

4. **Slaves not powered**: Verify slave power supplies

5. **Missing promiscuous mode**:
   ```bash
   sudo ip link set enp3s0 promisc on
   ```

#### Permission Denied

```
Error: Failed to initialize SOEM on enp3s0: PermissionDenied
```

**Solution**: Grant raw socket capability:
```bash
sudo setcap cap_net_raw+ep ./target/release/plc-daemon
```

Or run as root (not recommended for production).

#### Working Counter Mismatch

```
WARN plc_fieldbus::ethercat: Working counter mismatch expected=6 actual=4 cycle=1234
```

The Working Counter (WKC) indicates how many slaves successfully processed the frame. A mismatch means:

- A slave dropped off the network
- Cable disconnected during operation
- Slave power failure

**Solutions:**

1. Check physical connections
2. Verify slave power
3. Increase `wkc_error_threshold` for transient issues
4. Set `wkc_error_threshold = 0` to disable fault (only log warnings)

#### DC Synchronization Issues

```
WARN plc_fieldbus::dc_sync: No DC-capable slaves found
```

**Causes:**

1. Slaves do not support DC (check device documentation)
2. DC disabled in configuration
3. Older slave firmware without DC support

**Solutions:**

1. Verify slave DC capability
2. Update slave firmware if needed
3. Set `dc_enabled = false` if DC is not required

#### High Jitter / Cycle Overruns

```
WARN plc_runtime::scheduler: Cycle overrun cycle=5678 actual_us=1250 target_us=1000
```

**Solutions:**

1. Use PREEMPT_RT kernel (see [Real-Time Linux Setup](#real-time-linux-setup))
2. Isolate CPU cores for PLC process
3. Use recommended Intel NIC
4. Disable unnecessary system services
5. Increase cycle time if determinism is not critical

---

## Simulated I/O

For development and testing without hardware, use the simulated driver:

```toml
[fieldbus]
driver = "simulated"
```

The simulated driver:
- Stores I/O values in memory
- Echoes outputs to inputs (for basic testing)
- Supports setting simulated input values programmatically
- Works on all platforms without special privileges

### Simulated Driver in Code

```rust
use plc_fieldbus::{SimulatedDriver, FieldbusDriver, FieldbusInputs};

let mut driver = SimulatedDriver::new();
driver.init()?;

// Set simulated inputs
driver.set_simulated_inputs(FieldbusInputs {
    digital: 0xFF,
    analog: [100, 200, 300, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
});

// Exchange and read
driver.exchange()?;
let inputs = driver.get_inputs();

driver.shutdown()?;
```

---

## Real-Time Linux Setup

For production deployments requiring deterministic cycle times, configure a real-time Linux environment.

### 1. Install PREEMPT_RT Kernel

```bash
# Ubuntu/Debian
sudo apt install linux-image-rt-amd64

# Reboot into RT kernel
sudo reboot
```

Verify the kernel:
```bash
uname -r
# Should show: X.X.X-rt or similar
```

### 2. Configure CPU Isolation

Add to `/etc/default/grub`:

```
GRUB_CMDLINE_LINUX="isolcpus=2,3 nohz_full=2,3 rcu_nocbs=2,3"
```

Apply changes:
```bash
sudo update-grub
sudo reboot
```

### 3. Run with Real-Time Priority

```bash
# Run with FIFO scheduling at priority 90
sudo chrt -f 90 ./target/release/plc-daemon run \
    --config config/ethercat.toml \
    --wasm-module app.wasm
```

### 4. Memory Locking

The runtime automatically attempts to lock memory pages. Ensure sufficient limits:

```bash
# Check current limits
ulimit -l

# Set unlimited (requires root or capability)
ulimit -l unlimited
```

Or configure in `/etc/security/limits.conf`:
```
plc-user    soft    memlock    unlimited
plc-user    hard    memlock    unlimited
```

### 5. Verify Latency

Use `cyclictest` to verify system latency:

```bash
# Install rt-tests
sudo apt install rt-tests

# Run latency test
sudo cyclictest -p 90 -t1 -n -i 1000 -l 10000
```

**Target values:**
- Max latency < 50 us for 1 ms cycle time
- Max latency < 100 us for 2 ms cycle time

### 6. Docker Real-Time Configuration

For containerized deployments, use this `docker-compose.yml`:

```yaml
version: '3.8'
services:
  plc:
    image: virtual-plc:latest
    privileged: true
    network_mode: host
    cap_add:
      - SYS_NICE
      - IPC_LOCK
      - NET_RAW
    ulimits:
      rtprio: 99
      memlock: -1
    volumes:
      - ./config:/etc/plc
      - ./programs:/var/lib/plc/programs
```

---

## Further Reading

- [Architecture Overview](architecture.md)
- [ADR-001: Split-Architecture Runtime](adr/001-runtime-arch.md)
- [ADR-003: EtherCAT Primary Fieldbus](adr/003-ethercat-primary.md)
- [Process Image ABI](process-image-abi.md)
- [Getting Started Guide](getting-started.md)

### External Resources

- [EtherCAT Technology Group](https://www.ethercat.org/)
- [SOEM Documentation](https://github.com/OpenEtherCATsociety/SOEM)
- [Modbus Protocol Specification](https://modbus.org/specs.php)
- [Linux PREEMPT_RT](https://wiki.linuxfoundation.org/realtime/start)
