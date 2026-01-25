# ADR-003: EtherCAT as Primary Deterministic Fieldbus

**Status:** Accepted

**Date:** 2024-01-15

**Supersedes:** None

**Related:** [ADR-001](001-runtime-arch.md) (Split-Architecture Runtime)

## Context

A soft PLC requires a fieldbus to communicate with field devices (sensors, actuators, drives). The choice of fieldbus significantly impacts:
- Achievable cycle times
- Number of supported devices
- Hardware availability and cost
- Implementation complexity

### Problem Statement

Which industrial fieldbus should be the primary communication protocol for the Virtual PLC runtime?

### Constraints

1. Must support cycle times ≤1ms for motion control applications
2. Must support at least 64 I/O modules on a single network
3. Should have readily available, affordable hardware
4. Must have open-source or freely available protocol stacks
5. Should be widely adopted in industry

### Options Considered

**Option A: Modbus TCP**
- Pros: Simple, ubiquitous, easy implementation
- Cons: Non-deterministic (TCP/IP), limited to ~10ms cycles, polling-based

**Option B: PROFINET**
- Pros: Widely adopted, Siemens ecosystem
- Cons: Complex stack, licensing concerns, limited open implementations

**Option C: EtherNet/IP**
- Pros: Rockwell ecosystem, CIP protocol family
- Cons: Non-deterministic without add-ons, licensing

**Option D: EtherCAT**
- Pros: True real-time (<100µs possible), open protocol, efficient topology
- Cons: Requires dedicated NIC, master implementation complexity

**Option E: POWERLINK**
- Pros: Open-source stack (openPOWERLINK), deterministic
- Cons: Smaller ecosystem, less hardware variety

### Fieldbus Comparison

| Feature | EtherCAT | PROFINET IRT | EtherNet/IP | Modbus TCP |
|---------|----------|--------------|-------------|------------|
| Min Cycle | 31.25µs | 31.25µs | ~1ms | ~10ms |
| Determinism | Hardware | Hardware | Software | None |
| Topology | Line/Tree | Star/Line | Star | Star |
| Max Nodes | 65535 | ~256 | ~256 | 247 |
| Open Stack | SOEM, IgH | Limited | Limited | Many |
| Licensing | Free | Fees | Fees | Free |

## Decision

We adopt **EtherCAT** as the primary deterministic fieldbus, with **Modbus TCP** as a secondary option for legacy/simple deployments.

### Rationale

1. **Performance**: EtherCAT achieves sub-100µs cycle times, essential for motion control
2. **Efficiency**: "Processing on the fly" minimizes frame overhead
3. **Open Stack**: SOEM (Simple Open EtherCAT Master) is BSD-licensed
4. **Hardware**: Wide variety of EtherCAT slaves from multiple vendors
5. **Topology**: Flexible line/tree topology suits industrial layouts
6. **Distributed Clocks**: Nanosecond-level synchronization across devices

### Master Implementation

We support two EtherCAT master stacks:

| Stack | License | Features | Use Case |
|-------|---------|----------|----------|
| SOEM | BSD | Lightweight, portable | Default, embedded |
| IgH EtherLab | GPL | Full-featured, mature | Full installations |

The `EthercatTransport` trait (`plc-fieldbus/src/ethercat.rs`) abstracts the transport layer, allowing
different EtherCAT master implementations (SOEM, IgH, or simulated):

```rust
pub trait EthercatTransport: Send {
    /// Scan for slaves on the network
    fn scan_slaves(&mut self) -> PlcResult<Vec<SlaveConfig>>;

    /// Set all slaves to the specified state
    fn set_state(&mut self, state: SlaveState) -> PlcResult<()>;

    /// Configure DC for a slave
    fn configure_slave_dc(&mut self, config: &DcSlaveConfig) -> PlcResult<()>;

    /// Read the DC system time from the reference clock
    fn read_dc_time(&mut self) -> PlcResult<u64>;

    /// Exchange process data (send outputs, receive inputs)
    fn exchange(&mut self, outputs: &[u8], inputs: &mut [u8]) -> PlcResult<u16>;

    /// Read an SDO (Service Data Object)
    fn sdo_read(&mut self, request: &SdoRequest) -> PlcResult<Vec<u8>>;

    /// Write an SDO
    fn sdo_write(&mut self, request: &SdoRequest) -> PlcResult<()>;
}
```

The `EthercatMaster` struct implements `FieldbusDriver` and uses an `EthercatTransport` for
low-level communication.

## Implementation Details

### Network Topology

```
   Master (NIC)
       │
       ▼
   ┌───────┐     ┌───────┐     ┌───────┐
   │Slave 1│────►│Slave 2│────►│Slave 3│
   └───────┘     └───────┘     └───────┘
       │
       ▼
   ┌───────┐
   │Slave 4│ (branching)
   └───────┘
```

EtherCAT uses a logical ring topology over physical line/tree cabling.

### Frame Processing

```
┌────────────────────────────────────────────────────┐
│                EtherCAT Frame                      │
├────────┬────────┬────────┬────────┬───────────────┤
│Ethernet│EtherCAT│Datagram│Datagram│     ...       │
│ Header │ Header │   1    │   2    │               │
└────────┴────────┴────────┴────────┴───────────────┘
         │
         ▼
Each slave reads inputs, inserts them into frame,
reads outputs from frame, all in hardware
```

### Distributed Clocks

For synchronized motion, we configure DC:

```rust
// Configure 1ms cycle with 10µs tolerance
master.configure_dc(1_000_000)?; // 1ms in nanoseconds

// Slaves synchronize to DC reference clock
// Typical jitter: <1µs across all slaves
```

### I/O Mapping

ESI (EtherCAT Slave Information) files define slave capabilities:

```xml
<Device>
  <Name>EL1008</Name>
  <Inputs>
    <BitLength>8</BitLength>
  </Inputs>
</Device>
```

The driver parses ESI to auto-configure PDO mapping.

## Consequences

### Positive

1. **Performance**: Sub-millisecond cycles achievable
2. **Scalability**: Thousands of I/O points on single network
3. **Synchronization**: DC provides precise timing across devices
4. **Cost**: Competitive pricing for EtherCAT I/O modules
5. **Flexibility**: Line topology simplifies wiring

### Negative

1. **NIC Requirements**: Needs compatible Ethernet controller (Intel i210/i350 recommended)
2. **Complexity**: Master implementation more complex than Modbus
3. **Real-Time OS**: Requires PREEMPT_RT for reliable timing
4. **Debugging**: Protocol analysis requires specialized tools

### Hardware Recommendations

**Recommended NICs:**
- Intel i210 (single port)
- Intel i350 (dual port)
- Intel i225 (2.5GbE, newer)

**Avoid:**
- Realtek consumer NICs (timing issues)
- USB Ethernet adapters (latency)
- Virtual machine passthrough (unless SR-IOV)

### Modbus TCP Fallback

For simpler installations or legacy integration:

```rust
// Modbus TCP for non-critical I/O
let modbus = ModbusTcpDriver::new("192.168.1.100:502");

// Map registers to process image
modbus.map_holding_registers(0, 16); // 16 words
```

Modbus is suitable for:
- HMI communication
- PLC-to-PLC data exchange
- Non-time-critical I/O
- Legacy device integration

## Verification

1. **Timing Tests**: Verify cycle time with oscilloscope on DC sync signals
2. **Stress Tests**: Full I/O load for extended periods
3. **Fault Injection**: Cable disconnects, slave failures
4. **Latency Measurement**: cyclictest during EtherCAT traffic

## References

- [EtherCAT Technology Group](https://www.ethercat.org/)
- [SOEM (Simple Open EtherCAT Master)](https://github.com/OpenEtherCATsociety/SOEM)
- [IgH EtherCAT Master](https://etherlab.org/en/ethercat/)
- [EtherCAT Slave Information (ESI) Schema](https://www.ethercat.org/en/downloads.html)
- [Beckhoff EtherCAT I/O](https://www.beckhoff.com/en-us/products/i-o/)
