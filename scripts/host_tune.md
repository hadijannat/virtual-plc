# Host Tuning Guide for Virtual PLC

This guide covers the system configuration required to achieve deterministic real-time performance for the Virtual PLC runtime.

## Acceptance Criteria

- 99.999th percentile jitter < 50 microseconds under load
- Maximum latency < 100 microseconds
- Zero deadline overruns during 168-hour soak test

## Prerequisites

### 1. PREEMPT_RT Kernel

The PREEMPT_RT patch provides the lowest latency Linux kernel. This is highly recommended for production deployments.

#### Ubuntu/Debian

```bash
# Check if PREEMPT_RT is available
apt search linux-image.*rt

# Install (adjust version as needed)
sudo apt install linux-image-rt-amd64

# Reboot and select RT kernel from GRUB
sudo reboot
```

#### From Source

```bash
# Download kernel and RT patch (match versions!)
wget https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-6.6.tar.xz
wget https://cdn.kernel.org/pub/linux/kernel/projects/rt/6.6/patch-6.6-rt15.patch.xz

# Extract and patch
tar xf linux-6.6.tar.xz
cd linux-6.6
xzcat ../patch-6.6-rt15.patch.xz | patch -p1

# Configure (start from current config)
cp /boot/config-$(uname -r) .config
make oldconfig

# Enable PREEMPT_RT
# In menuconfig: General setup -> Preemption Model -> Fully Preemptible Kernel (RT)
make menuconfig

# Build and install
make -j$(nproc)
sudo make modules_install
sudo make install
sudo update-grub
```

#### Verify Installation

```bash
# Check for PREEMPT_RT
uname -a | grep -i "preempt.*rt"

# Or check /proc/version
cat /proc/version | grep -i "preempt.*rt"
```

### 2. GRUB Kernel Parameters

Edit `/etc/default/grub` and add these parameters to `GRUB_CMDLINE_LINUX_DEFAULT`:

```bash
# Isolate CPUs for RT tasks (adjust CPU numbers for your system)
# CPUs 2-3 will be reserved for the PLC runtime
isolcpus=2-3

# Prevent kernel threads from running on isolated CPUs
nohz_full=2-3

# Disable RCU callbacks on isolated CPUs
rcu_nocbs=2-3

# Disable IRQ balancer on isolated CPUs
irqaffinity=0-1

# Disable CPU frequency scaling (use performance governor)
intel_pstate=disable
processor.max_cstate=1
idle=poll

# Disable speculative execution mitigations (security vs performance trade-off)
# WARNING: Only use in isolated, trusted environments
# mitigations=off

# Disable transparent huge pages
transparent_hugepage=never

# Disable NUMA balancing
numa_balancing=disable
```

Apply changes:

```bash
sudo update-grub
sudo reboot
```

### 3. CPU Frequency Scaling

Lock CPUs to maximum frequency:

```bash
# Check current governor
cat /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor

# Set performance governor
for cpu in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do
    echo performance | sudo tee $cpu
done

# Verify
cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor
```

For persistent settings, create `/etc/systemd/system/cpu-performance.service`:

```ini
[Unit]
Description=Set CPU governor to performance

[Service]
Type=oneshot
ExecStart=/bin/sh -c 'for gov in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do echo performance > $gov; done'
RemainAfterExit=yes

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl enable cpu-performance
sudo systemctl start cpu-performance
```

### 4. IRQ Affinity

Move interrupts away from isolated CPUs:

```bash
# View current IRQ affinity
cat /proc/interrupts

# Move all IRQs to CPUs 0-1 (non-isolated)
for irq in /proc/irq/*/smp_affinity; do
    echo 3 | sudo tee $irq 2>/dev/null || true
done

# Or use irqbalance with isolation
sudo systemctl stop irqbalance
sudo systemctl disable irqbalance
```

For the network card (EtherCAT interface), pin to a specific CPU:

```bash
# Find the IRQ for your network interface
cat /proc/interrupts | grep eth0

# Pin to CPU 1 (bitmask: 0x2)
echo 2 | sudo tee /proc/irq/<IRQ_NUMBER>/smp_affinity
```

### 5. Memory Locking

Allow the PLC process to lock memory:

Edit `/etc/security/limits.conf`:

```
# Allow plc user to lock unlimited memory
plc     soft    memlock    unlimited
plc     hard    memlock    unlimited

# Or for all users in realtime group
@realtime   soft    memlock    unlimited
@realtime   hard    memlock    unlimited
```

Create realtime group and add user:

```bash
sudo groupadd realtime
sudo usermod -aG realtime plc
```

### 6. Real-Time Priority

Allow setting RT priority:

Edit `/etc/security/limits.conf`:

```
# Allow plc user to set RT priority
plc     soft    rtprio     99
plc     hard    rtprio     99

# Or for realtime group
@realtime   soft    rtprio     99
@realtime   hard    rtprio     99
```

### 7. Kernel Tuning

Create `/etc/sysctl.d/99-realtime.conf`:

```bash
# Minimize swapping
vm.swappiness = 0

# Disable watchdog
kernel.nmi_watchdog = 0
kernel.soft_watchdog = 0
kernel.watchdog = 0

# Network tuning for EtherCAT
net.core.rmem_max = 16777216
net.core.wmem_max = 16777216
net.core.rmem_default = 16777216
net.core.wmem_default = 16777216
net.core.netdev_budget = 600
net.core.netdev_budget_usecs = 8000

# Disable IPv6 if not needed
net.ipv6.conf.all.disable_ipv6 = 1
net.ipv6.conf.default.disable_ipv6 = 1
```

Apply:

```bash
sudo sysctl -p /etc/sysctl.d/99-realtime.conf
```

### 8. Disable Unnecessary Services

```bash
# Disable services that can cause latency spikes
sudo systemctl disable irqbalance
sudo systemctl disable ondemand
sudo systemctl disable cpufrequtils
sudo systemctl disable thermald

# Disable automatic updates during runtime
sudo systemctl disable apt-daily.timer
sudo systemctl disable apt-daily-upgrade.timer
```

## Verification

### 1. Run Cyclictest

```bash
# Install rt-tests
sudo apt install rt-tests stress-ng

# Quick baseline test
sudo cyclictest --mlockall --priority=99 --interval=1000 \
    --duration=60 --histogram=1000 --histfile=/tmp/hist.txt

# Full verification with load
./scripts/verify_latency.sh --full
```

### 2. Expected Results

| Metric | Acceptable | Good | Excellent |
|--------|------------|------|-----------|
| Max latency | < 100 microseconds | < 50 microseconds | < 20 microseconds |
| P99.999 | < 50 microseconds | < 30 microseconds | < 15 microseconds |
| Average | < 10 microseconds | < 5 microseconds | < 3 microseconds |

### 3. Common Issues

**High latency spikes:**
- Check if PREEMPT_RT kernel is running
- Verify CPU isolation with `cat /sys/devices/system/cpu/isolated`
- Check for IRQs on isolated CPUs: `cat /proc/interrupts`

**Inconsistent results:**
- Disable SMT/Hyperthreading in BIOS
- Check CPU frequency: `cat /proc/cpuinfo | grep MHz`
- Ensure no thermal throttling: `sensors`

**Memory allocation failures:**
- Check memlock limits: `ulimit -l`
- Verify available memory: `free -h`

## Docker Configuration

For running Virtual PLC in Docker with RT capabilities:

```yaml
# docker-compose.yml
services:
  plc:
    image: virtual-plc:latest
    privileged: true
    cap_add:
      - SYS_NICE
      - IPC_LOCK
      - NET_RAW
    ulimits:
      rtprio: 99
      memlock: -1
    cpuset: "2-3"  # Isolated CPUs
    environment:
      - PLC_CPU_AFFINITY=2
```

## Troubleshooting

### Check RT Capabilities

```bash
# Verify process can set RT priority
chrt -f 99 sleep 1

# Check if mlockall works
./target/release/plc-daemon --dry-run
```

### Monitor Latency

```bash
# Real-time latency histogram
sudo cyclictest -m -p99 -i1000 -h1000 -D60

# Watch for latency spikes
sudo trace-cmd record -e sched_switch -e irq_handler_entry
```

### Performance Analysis

```bash
# CPU usage by core
mpstat -P ALL 1

# Context switches
vmstat 1

# IRQ statistics
watch -n1 cat /proc/interrupts
```

## References

- [PREEMPT_RT Wiki](https://wiki.linuxfoundation.org/realtime/start)
- [Red Hat RT Tuning Guide](https://access.redhat.com/documentation/en-us/red_hat_enterprise_linux_for_real_time)
- [cyclictest Documentation](https://wiki.linuxfoundation.org/realtime/documentation/howto/tools/cyclictest)
- [EtherCAT Linux](https://ethercat.org/en/downloads.html)
