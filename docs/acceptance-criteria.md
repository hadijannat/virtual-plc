# Production Acceptance Criteria

This document defines the acceptance criteria for production deployment of vPLC.

## Real-Time Performance Metrics

| Metric | Definition | Target | Measurement Method |
|--------|------------|--------|-------------------|
| **Wake-up jitter** | `abs(actual_wakeup - scheduled_wakeup)` | p99.9 < 50 µs | `cyclictest` with histogram |
| **Cycle execution time** | `end_of_cycle - actual_wakeup` | < `cycle_time` | Runtime metrics histogram |
| **Deadline miss** | `end_of_cycle > scheduled_wakeup + cycle_time` | 0 allowed | Runtime counter |
| **Overrun** | `end_of_cycle - scheduled_wakeup - cycle_time` | < `max_overrun` | Runtime metrics |

### Metric Definitions

**Wake-up jitter**: The difference between when the RT thread was scheduled to wake and when it actually woke. This is primarily influenced by kernel scheduling latency and is measured using `cyclictest`.

**Cycle execution time**: The wall-clock time from the start of a scan cycle (after wakeup) to its completion (before sleep). This includes:
- Input sampling from fieldbus
- Wasm logic execution
- Output publication to fieldbus

**Deadline miss**: A cycle that completes after its scheduled deadline. Any deadline miss is a failure condition in hard real-time systems.

**Overrun**: The amount by which a cycle exceeds its allocated time. Small overruns may be tolerable in soft real-time systems up to `max_overrun`.

## Real-Time Environment Checklist

Before deploying to production, verify the following:

### Kernel Configuration

- [ ] **PREEMPT_RT kernel** installed and booted
  ```bash
  uname -a | grep -i preempt
  cat /proc/version | grep -i preempt
  ```

- [ ] **CPU isolation** configured via `isolcpus` boot parameter
  ```bash
  cat /proc/cmdline | grep isolcpus
  ```

- [ ] **Timer tick isolation** via `nohz_full` (optional but recommended)
  ```bash
  cat /proc/cmdline | grep nohz_full
  ```

### Process Capabilities

- [ ] **CAP_SYS_NICE** - Required for SCHED_FIFO/SCHED_RR
  ```bash
  getpcaps $PID | grep sys_nice
  ```

- [ ] **CAP_IPC_LOCK** - Required for mlockall()
  ```bash
  getpcaps $PID | grep ipc_lock
  ```

- [ ] **CAP_NET_RAW** - Required for EtherCAT raw socket access
  ```bash
  getpcaps $PID | grep net_raw
  ```

### Resource Limits

- [ ] **RLIMIT_RTPRIO** >= 99
  ```bash
  ulimit -r
  ```

- [ ] **RLIMIT_MEMLOCK** = unlimited
  ```bash
  ulimit -l
  ```

### Hardware Configuration

- [ ] **NIC timestamping** enabled for EtherCAT interface (if applicable)
- [ ] **Dedicated network interface** for fieldbus (not shared with other traffic)
- [ ] **CPU frequency scaling** disabled or set to performance governor
  ```bash
  cat /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor
  ```

## Validation Procedure

### 1. Latency Validation

Run `cyclictest` for at least 24 hours under realistic load:

```bash
cyclictest -p 99 -t 1 -n -i 1000 -l 86400000 -a <isolated_cpu> -h 100
```

**Pass criteria**: Max latency < 100 µs, p99.9 < 50 µs

### 2. Runtime Validation

Run vPLC with simulated I/O for at least 24 hours:

```bash
cargo run -p plc-daemon --release -- \
  --simulated \
  --config config/production.toml \
  --max-cycles 0
```

Monitor metrics endpoint and verify:
- Zero deadline misses
- p99.9 execution time < cycle_time * 0.8

### 3. Fault Injection Testing

Verify graceful degradation under:
- [ ] Wasm module panic/trap
- [ ] Fieldbus communication loss
- [ ] CPU overload (stress-ng)
- [ ] Memory pressure

### 4. Integration Testing

With actual fieldbus hardware:
- [ ] Verify I/O mapping correctness
- [ ] Verify Distributed Clock synchronization (EtherCAT)
- [ ] Verify watchdog triggers on communication loss
- [ ] Verify safe output state on fault

## Production Configuration

Recommended `production.toml` settings:

```toml
cycle_time = "1ms"
watchdog_timeout = "3ms"
max_overrun = "500us"

[realtime]
enabled = true
policy = "fifo"
priority = 90
cpu_affinity = 2      # Use isolated CPU
lock_memory = true
prefault_stack_size = 8388608
fail_fast = true      # Fail immediately if RT requirements not met

[fault_policy]
on_overrun = "fault"  # Strict: fault on any overrun
safe_outputs = "all_off"
fault_latch = true    # Require manual reset

[metrics]
enabled = true
histogram_size = 100000
percentiles = [50.0, 90.0, 99.0, 99.9, 99.99]
http_export = true
http_port = 9090
```

## Sign-Off Checklist

Before production deployment:

- [ ] All latency validation tests pass
- [ ] 24-hour runtime validation complete with zero deadline misses
- [ ] Fault injection testing complete
- [ ] Hardware integration testing complete
- [ ] Safety system interlocks verified independent of vPLC
- [ ] Rollback procedure documented and tested
- [ ] Monitoring and alerting configured
- [ ] Incident response procedure documented
