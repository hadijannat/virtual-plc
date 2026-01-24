//! EtherCAT Distributed Clocks (DC) synchronization.
//!
//! Provides:
//! - DC configuration and initialization
//! - System time distribution
//! - Drift compensation
//! - Sync0/Sync1 event management
//!
//! DC enables synchronized outputs across all slaves with sub-microsecond precision.

use plc_common::error::PlcResult;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tracing::{debug, info, trace, warn};

/// DC register addresses in ESC memory.
pub mod registers {
    /// Port 0 receive time (64-bit).
    pub const PORT0_RECV_TIME: u16 = 0x0900;
    /// Port 1 receive time.
    pub const PORT1_RECV_TIME: u16 = 0x0904;
    /// Port 2 receive time.
    pub const PORT2_RECV_TIME: u16 = 0x0908;
    /// Port 3 receive time.
    pub const PORT3_RECV_TIME: u16 = 0x090C;

    /// System time (64-bit local copy of distributed clock).
    pub const SYSTEM_TIME: u16 = 0x0910;
    /// Receive time port 0 (64-bit).
    pub const RECV_TIME_PORT0: u16 = 0x0918;
    /// System time offset.
    pub const SYSTEM_TIME_OFFSET: u16 = 0x0920;
    /// System time delay.
    pub const SYSTEM_TIME_DELAY: u16 = 0x0928;
    /// System time difference.
    pub const SYSTEM_TIME_DIFF: u16 = 0x092C;

    /// DC control loop filter depth (0x0930:0x0931).
    pub const DC_FILTER_DEPTH: u16 = 0x0930;

    /// Cyclic unit control (DC activation).
    pub const DC_CYCLIC_UNIT_CTRL: u16 = 0x0980;
    /// Sync0 cycle time.
    pub const DC_SYNC0_CYCLE: u16 = 0x09A0;
    /// Sync1 cycle time.
    pub const DC_SYNC1_CYCLE: u16 = 0x09A4;

    /// DC activation register.
    pub const DC_ACTIVATION: u16 = 0x0981;
}

/// DC synchronization mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DcSyncMode {
    /// DC disabled.
    #[default]
    Disabled,
    /// Free-run mode (no synchronization).
    FreeRun,
    /// SM-synchronous mode (sync to mailbox).
    SmSync,
    /// DC synchronous mode with Sync0.
    DcSync0,
    /// DC synchronous mode with Sync0 and Sync1.
    DcSync01,
}

impl DcSyncMode {
    /// Get the activation register value for this mode.
    pub fn activation_value(&self) -> u8 {
        match self {
            Self::Disabled | Self::FreeRun => 0x00,
            Self::SmSync => 0x00,
            Self::DcSync0 => 0x01,
            Self::DcSync01 => 0x03,
        }
    }
}

/// DC configuration for a single slave.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DcSlaveConfig {
    /// Slave position.
    pub position: u16,
    /// Whether DC is supported by this slave.
    pub dc_supported: bool,
    /// Sync mode.
    pub sync_mode: DcSyncMode,
    /// Sync0 cycle time in nanoseconds.
    pub sync0_cycle_ns: u64,
    /// Sync1 cycle time in nanoseconds (if using Sync01).
    pub sync1_cycle_ns: u64,
    /// Sync0 shift time (offset from cycle start).
    pub sync0_shift_ns: i32,
    /// Propagation delay from master in nanoseconds.
    pub propagation_delay_ns: u32,
    /// System time offset for this slave.
    pub system_time_offset: i64,
    /// Whether this slave is the reference clock.
    pub is_reference_clock: bool,
}

impl DcSlaveConfig {
    /// Create a new DC slave configuration.
    pub fn new(position: u16) -> Self {
        Self {
            position,
            dc_supported: false,
            sync_mode: DcSyncMode::Disabled,
            sync0_cycle_ns: 0,
            sync1_cycle_ns: 0,
            sync0_shift_ns: 0,
            propagation_delay_ns: 0,
            system_time_offset: 0,
            is_reference_clock: false,
        }
    }

    /// Configure for Sync0 mode.
    pub fn with_sync0(mut self, cycle_ns: u64) -> Self {
        self.dc_supported = true;
        self.sync_mode = DcSyncMode::DcSync0;
        self.sync0_cycle_ns = cycle_ns;
        self
    }

    /// Configure for Sync0+Sync1 mode.
    pub fn with_sync01(mut self, sync0_cycle_ns: u64, sync1_cycle_ns: u64) -> Self {
        self.dc_supported = true;
        self.sync_mode = DcSyncMode::DcSync01;
        self.sync0_cycle_ns = sync0_cycle_ns;
        self.sync1_cycle_ns = sync1_cycle_ns;
        self
    }

    /// Set the Sync0 shift time.
    pub fn with_shift(mut self, shift_ns: i32) -> Self {
        self.sync0_shift_ns = shift_ns;
        self
    }
}

/// DC system time management.
#[derive(Debug)]
pub struct DcSystemTime {
    /// Reference time at initialization.
    reference_instant: Instant,
    /// DC time at reference instant (nanoseconds).
    dc_time_at_reference: u64,
    /// Accumulated drift correction.
    drift_correction_ns: i64,
    /// Last measured DC time.
    last_dc_time: u64,
    /// Master cycle count.
    cycle_count: u64,
}

impl Default for DcSystemTime {
    fn default() -> Self {
        Self::new()
    }
}

impl DcSystemTime {
    /// Create a new DC system time manager.
    pub fn new() -> Self {
        Self {
            reference_instant: Instant::now(),
            dc_time_at_reference: 0,
            drift_correction_ns: 0,
            last_dc_time: 0,
            cycle_count: 0,
        }
    }

    /// Initialize with the current DC time from the reference clock.
    pub fn initialize(&mut self, dc_time_ns: u64) {
        self.reference_instant = Instant::now();
        self.dc_time_at_reference = dc_time_ns;
        self.last_dc_time = dc_time_ns;
        self.drift_correction_ns = 0;
        self.cycle_count = 0;
        info!(dc_time_ns, "DC system time initialized");
    }

    /// Get the expected DC time for now.
    pub fn expected_dc_time(&self) -> u64 {
        let elapsed = self.reference_instant.elapsed();

        // Use i128 for intermediate math to safely handle:
        // - Large DC times (up to 2^64 ns ≈ 584 years)
        // - Negative drift corrections
        // - Addition without overflow
        let base_time = self.dc_time_at_reference as i128;
        let elapsed_ns = elapsed.as_nanos() as i128;
        let drift = self.drift_correction_ns as i128;

        let expected = base_time + elapsed_ns + drift;

        // Clamp to valid u64 range (DC time can't be negative)
        if expected < 0 {
            0
        } else if expected > u64::MAX as i128 {
            u64::MAX
        } else {
            expected as u64
        }
    }

    /// Update with a measured DC time from the reference clock.
    ///
    /// Returns the measured drift in nanoseconds.
    pub fn update(&mut self, measured_dc_time: u64) -> i64 {
        let expected = self.expected_dc_time();
        let drift = measured_dc_time.wrapping_sub(expected) as i64;

        // Apply a simple low-pass filter to the drift correction
        // This prevents sudden jumps while tracking long-term drift
        const FILTER_SHIFT: i64 = 4; // Divide by 16
        self.drift_correction_ns += drift >> FILTER_SHIFT;

        self.last_dc_time = measured_dc_time;
        self.cycle_count += 1;

        trace!(
            expected,
            measured = measured_dc_time,
            drift,
            correction = self.drift_correction_ns,
            "DC time update"
        );

        drift
    }

    /// Get the current cycle count.
    pub fn cycle_count(&self) -> u64 {
        self.cycle_count
    }

    /// Get the accumulated drift correction.
    pub fn drift_correction(&self) -> i64 {
        self.drift_correction_ns
    }
}

/// DC synchronization statistics.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DcSyncStats {
    /// Number of sync cycles.
    pub sync_cycles: u64,
    /// Minimum observed deviation in nanoseconds.
    pub min_deviation_ns: i64,
    /// Maximum observed deviation in nanoseconds.
    pub max_deviation_ns: i64,
    /// Sum of deviations for mean calculation.
    pub sum_deviation_ns: i64,
    /// Number of sync errors (deviation > threshold).
    pub sync_errors: u64,
    /// Error threshold in nanoseconds.
    pub error_threshold_ns: i64,
}

impl DcSyncStats {
    /// Create new stats with the given error threshold.
    pub fn new(error_threshold_ns: i64) -> Self {
        Self {
            min_deviation_ns: i64::MAX,
            max_deviation_ns: i64::MIN,
            error_threshold_ns,
            ..Default::default()
        }
    }

    /// Record a deviation measurement.
    pub fn record(&mut self, deviation_ns: i64) {
        self.sync_cycles += 1;
        self.min_deviation_ns = self.min_deviation_ns.min(deviation_ns);
        self.max_deviation_ns = self.max_deviation_ns.max(deviation_ns);
        self.sum_deviation_ns = self.sum_deviation_ns.wrapping_add(deviation_ns);

        if deviation_ns.abs() > self.error_threshold_ns {
            self.sync_errors += 1;
        }
    }

    /// Get the mean deviation.
    pub fn mean_deviation_ns(&self) -> Option<i64> {
        if self.sync_cycles > 0 {
            Some(self.sum_deviation_ns / self.sync_cycles as i64)
        } else {
            None
        }
    }

    /// Get the peak-to-peak jitter.
    pub fn jitter_ns(&self) -> Option<i64> {
        if self.sync_cycles > 0 && self.min_deviation_ns != i64::MAX {
            Some(self.max_deviation_ns - self.min_deviation_ns)
        } else {
            None
        }
    }

    /// Reset statistics.
    pub fn reset(&mut self) {
        let threshold = self.error_threshold_ns;
        *self = Self::new(threshold);
    }
}

/// DC synchronization controller.
#[derive(Debug)]
pub struct DcController {
    /// Slave configurations.
    slaves: Vec<DcSlaveConfig>,
    /// Reference clock slave position.
    reference_clock: Option<u16>,
    /// System time manager.
    system_time: DcSystemTime,
    /// Sync statistics.
    stats: DcSyncStats,
    /// Master cycle time in nanoseconds.
    cycle_time_ns: u64,
    /// Whether DC is active.
    active: bool,
}

impl DcController {
    /// Create a new DC controller.
    pub fn new(cycle_time: Duration) -> Self {
        Self {
            slaves: Vec::new(),
            reference_clock: None,
            system_time: DcSystemTime::new(),
            stats: DcSyncStats::new(1000), // 1µs default threshold
            cycle_time_ns: cycle_time.as_nanos() as u64,
            active: false,
        }
    }

    /// Add a slave to the DC configuration.
    pub fn add_slave(&mut self, config: DcSlaveConfig) {
        // First DC-capable slave becomes the reference clock
        if config.dc_supported && self.reference_clock.is_none() {
            let mut config = config;
            config.is_reference_clock = true;
            self.reference_clock = Some(config.position);
            self.slaves.push(config);
        } else {
            self.slaves.push(config);
        }
    }

    /// Get the reference clock slave position.
    pub fn reference_clock(&self) -> Option<u16> {
        self.reference_clock
    }

    /// Initialize DC synchronization.
    pub fn initialize(&mut self, initial_dc_time: u64) -> PlcResult<()> {
        if self.reference_clock.is_none() {
            warn!("No DC-capable slaves found, DC synchronization disabled");
            self.active = false;
            return Ok(());
        }

        self.system_time.initialize(initial_dc_time);
        self.stats.reset();
        self.active = true;

        info!(
            reference_clock = self.reference_clock,
            cycle_time_ns = self.cycle_time_ns,
            slave_count = self.slaves.len(),
            "DC synchronization initialized"
        );

        Ok(())
    }

    /// Update DC synchronization with a measured time from the reference clock.
    pub fn update(&mut self, measured_dc_time: u64) -> i64 {
        if !self.active {
            return 0;
        }

        let deviation = self.system_time.update(measured_dc_time);
        self.stats.record(deviation);

        deviation
    }

    /// Get sync statistics.
    pub fn stats(&self) -> &DcSyncStats {
        &self.stats
    }

    /// Check if DC is active.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Get slave configurations.
    pub fn slaves(&self) -> &[DcSlaveConfig] {
        &self.slaves
    }

    /// Calculate propagation delays for all slaves.
    ///
    /// This should be called during initialization after reading
    /// port receive times from all slaves.
    pub fn calculate_propagation_delays(&mut self, port_times: &[(u16, [u32; 4])]) {
        // Simplified propagation delay calculation
        // In production, this would use the receive time differences
        // between adjacent slaves to calculate cable delays

        for (position, times) in port_times {
            if let Some(slave) = self.slaves.iter_mut().find(|s| s.position == *position) {
                // Calculate delay from first port's receive time
                // This is a simplification - real implementation would
                // account for the ring topology
                slave.propagation_delay_ns = times[0];
                debug!(
                    position,
                    delay_ns = slave.propagation_delay_ns,
                    "Calculated propagation delay"
                );
            }
        }
    }

    /// Get the DC configuration parameters for a slave.
    pub fn get_slave_config(&self, position: u16) -> Option<&DcSlaveConfig> {
        self.slaves.iter().find(|s| s.position == position)
    }

    /// Clear all slaves and reset DC state.
    ///
    /// This should be called before re-scanning the network to ensure
    /// stale DC configuration is not retained.
    pub fn clear(&mut self) {
        self.slaves.clear();
        self.reference_clock = None;
        self.active = false;
        self.stats.reset();
        self.system_time = DcSystemTime::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dc_sync_mode_activation() {
        assert_eq!(DcSyncMode::Disabled.activation_value(), 0x00);
        assert_eq!(DcSyncMode::DcSync0.activation_value(), 0x01);
        assert_eq!(DcSyncMode::DcSync01.activation_value(), 0x03);
    }

    #[test]
    fn test_dc_slave_config() {
        let config = DcSlaveConfig::new(0)
            .with_sync0(1_000_000)
            .with_shift(50_000);

        assert!(config.dc_supported);
        assert_eq!(config.sync_mode, DcSyncMode::DcSync0);
        assert_eq!(config.sync0_cycle_ns, 1_000_000);
        assert_eq!(config.sync0_shift_ns, 50_000);
    }

    #[test]
    fn test_dc_system_time() {
        let mut sys_time = DcSystemTime::new();
        sys_time.initialize(1_000_000_000); // 1 second in ns

        // Immediate expected time should be close to initialization
        let expected = sys_time.expected_dc_time();
        assert!(expected >= 1_000_000_000);
        assert!(expected < 1_100_000_000); // Within 100ms
    }

    #[test]
    fn test_dc_system_time_negative_drift() {
        let mut sys_time = DcSystemTime::new();
        sys_time.initialize(1_000_000_000); // 1 second in ns

        // Simulate negative drift correction (clock running slow)
        sys_time.drift_correction_ns = -500_000; // -500µs

        // Expected time should be less than base time + elapsed
        let expected = sys_time.expected_dc_time();
        // Should be approximately 1_000_000_000 - 500_000 = 999_500_000
        // (plus a tiny elapsed time from test execution)
        assert!(expected >= 999_000_000); // Within reasonable bounds
        assert!(expected < 1_000_000_000 + 100_000_000); // Not huge from overflow
    }

    #[test]
    fn test_dc_system_time_negative_result_clamps() {
        let mut sys_time = DcSystemTime::new();
        sys_time.initialize(100); // Very small initial time

        // Apply a large negative drift that would make the result negative
        sys_time.drift_correction_ns = -1_000_000_000; // -1 second

        // Result should clamp to 0, not wrap around to a huge number
        let expected = sys_time.expected_dc_time();
        assert_eq!(expected, 0);
    }

    #[test]
    fn test_dc_sync_stats() {
        let mut stats = DcSyncStats::new(100);

        stats.record(50);
        stats.record(-30);
        stats.record(80);

        assert_eq!(stats.sync_cycles, 3);
        assert_eq!(stats.min_deviation_ns, -30);
        assert_eq!(stats.max_deviation_ns, 80);
        assert_eq!(stats.jitter_ns(), Some(110));
        assert_eq!(stats.sync_errors, 0);
    }

    #[test]
    fn test_dc_sync_stats_errors() {
        let mut stats = DcSyncStats::new(100);

        stats.record(50);
        stats.record(150); // Exceeds threshold
        stats.record(-200); // Exceeds threshold

        assert_eq!(stats.sync_errors, 2);
    }

    #[test]
    fn test_dc_controller_reference_clock() {
        let mut dc = DcController::new(Duration::from_millis(1));

        // Non-DC slave doesn't become reference
        dc.add_slave(DcSlaveConfig::new(0));
        assert!(dc.reference_clock().is_none());

        // First DC-capable slave becomes reference
        dc.add_slave(DcSlaveConfig::new(1).with_sync0(1_000_000));
        assert_eq!(dc.reference_clock(), Some(1));

        // Second DC-capable slave doesn't change reference
        dc.add_slave(DcSlaveConfig::new(2).with_sync0(1_000_000));
        assert_eq!(dc.reference_clock(), Some(1));
    }

    #[test]
    fn test_dc_controller_active_flag_without_reference_clock() {
        let mut dc = DcController::new(Duration::from_millis(1));

        // No DC-capable slaves added
        dc.add_slave(DcSlaveConfig::new(0)); // Non-DC slave
        assert!(dc.reference_clock().is_none());
        assert!(!dc.is_active());

        // Initialize without reference clock should explicitly set active = false
        dc.initialize(1_000_000).unwrap();
        assert!(!dc.is_active());
    }

    #[test]
    fn test_dc_controller_reinitialize_without_reference_clock() {
        let mut dc = DcController::new(Duration::from_millis(1));

        // Add DC-capable slave and initialize
        dc.add_slave(DcSlaveConfig::new(0).with_sync0(1_000_000));
        dc.initialize(1_000_000).unwrap();
        assert!(dc.is_active());

        // Clear and add only non-DC slave
        dc.clear();
        dc.add_slave(DcSlaveConfig::new(0)); // Non-DC slave
        assert!(dc.reference_clock().is_none());

        // Re-initialize should deactivate DC
        dc.initialize(1_000_000).unwrap();
        assert!(!dc.is_active());
    }
}
