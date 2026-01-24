//! Cycle metrics collection for latency monitoring.
//!
//! Provides a ring buffer-based histogram for tracking cycle times
//! without heap allocations during normal operation.

use std::time::Duration;

/// Cycle execution metrics with ring buffer for latency tracking.
#[derive(Debug)]
pub struct CycleMetrics {
    /// Ring buffer of cycle durations in nanoseconds.
    samples: Box<[u64]>,
    /// Current write position in the ring buffer.
    write_pos: usize,
    /// Number of samples collected (saturates at buffer size).
    sample_count: usize,
    /// Total cycles executed.
    total_cycles: u64,
    /// Minimum observed cycle time in nanoseconds.
    min_ns: u64,
    /// Maximum observed cycle time in nanoseconds.
    max_ns: u64,
    /// Sum of all cycle times for mean calculation.
    sum_ns: u64,
    /// Number of cycle overruns detected.
    overrun_count: u64,
    /// Configured cycle deadline in nanoseconds.
    deadline_ns: u64,
}

impl CycleMetrics {
    /// Create a new metrics collector with the given histogram size.
    ///
    /// # Arguments
    ///
    /// * `histogram_size` - Number of samples to retain in the ring buffer.
    /// * `cycle_deadline` - Expected cycle time; cycles exceeding this are overruns.
    #[must_use]
    pub fn new(histogram_size: usize, cycle_deadline: Duration) -> Self {
        let size = histogram_size.max(1);
        Self {
            samples: vec![0u64; size].into_boxed_slice(),
            write_pos: 0,
            sample_count: 0,
            total_cycles: 0,
            min_ns: u64::MAX,
            max_ns: 0,
            sum_ns: 0,
            overrun_count: 0,
            deadline_ns: cycle_deadline.as_nanos() as u64,
        }
    }

    /// Record a cycle execution time.
    ///
    /// This method is designed to be allocation-free for use in RT context.
    pub fn record(&mut self, duration: Duration) {
        let ns = duration.as_nanos() as u64;

        // Update ring buffer
        self.samples[self.write_pos] = ns;
        self.write_pos = (self.write_pos + 1) % self.samples.len();
        self.sample_count = self.sample_count.saturating_add(1).min(self.samples.len());

        // Update statistics
        self.total_cycles += 1;
        self.min_ns = self.min_ns.min(ns);
        self.max_ns = self.max_ns.max(ns);
        self.sum_ns = self.sum_ns.wrapping_add(ns);

        // Track overruns
        if ns > self.deadline_ns {
            self.overrun_count += 1;
        }
    }

    /// Record a cycle time in nanoseconds directly.
    ///
    /// Avoids Duration construction overhead in tight loops.
    pub fn record_ns(&mut self, ns: u64) {
        self.samples[self.write_pos] = ns;
        self.write_pos = (self.write_pos + 1) % self.samples.len();
        self.sample_count = self.sample_count.saturating_add(1).min(self.samples.len());

        self.total_cycles += 1;
        self.min_ns = self.min_ns.min(ns);
        self.max_ns = self.max_ns.max(ns);
        self.sum_ns = self.sum_ns.wrapping_add(ns);

        if ns > self.deadline_ns {
            self.overrun_count += 1;
        }
    }

    /// Get total number of cycles executed.
    #[must_use]
    pub fn total_cycles(&self) -> u64 {
        self.total_cycles
    }

    /// Get minimum observed cycle time.
    #[must_use]
    pub fn min(&self) -> Option<Duration> {
        if self.total_cycles > 0 {
            Some(Duration::from_nanos(self.min_ns))
        } else {
            None
        }
    }

    /// Get maximum observed cycle time.
    #[must_use]
    pub fn max(&self) -> Option<Duration> {
        if self.total_cycles > 0 {
            Some(Duration::from_nanos(self.max_ns))
        } else {
            None
        }
    }

    /// Get mean cycle time.
    #[must_use]
    pub fn mean(&self) -> Option<Duration> {
        if self.total_cycles > 0 {
            Some(Duration::from_nanos(self.sum_ns / self.total_cycles))
        } else {
            None
        }
    }

    /// Get number of cycle overruns.
    #[must_use]
    pub fn overrun_count(&self) -> u64 {
        self.overrun_count
    }

    /// Compute a percentile from the ring buffer.
    ///
    /// # Arguments
    ///
    /// * `percentile` - Percentile to compute (0.0 to 100.0).
    ///
    /// Returns `None` if no samples have been collected or if percentile is out of range.
    #[must_use]
    pub fn percentile(&self, percentile: f64) -> Option<Duration> {
        if self.sample_count == 0 {
            return None;
        }

        // Validate percentile range
        if percentile < 0.0 || percentile > 100.0 || percentile.is_nan() {
            return None;
        }

        // Copy and sort samples
        let mut sorted: Vec<u64> = self.samples[..self.sample_count].to_vec();
        sorted.sort_unstable();

        let idx = ((percentile / 100.0) * (sorted.len() - 1) as f64).round() as usize;
        let idx = idx.min(sorted.len() - 1);

        Some(Duration::from_nanos(sorted[idx]))
    }

    /// Compute multiple percentiles efficiently.
    ///
    /// # Arguments
    ///
    /// * `percentiles` - Slice of percentiles to compute (0.0 to 100.0).
    ///
    /// Returns a vector of (percentile, duration) pairs.
    /// Invalid percentiles (< 0, > 100, or NaN) are skipped.
    #[must_use]
    pub fn percentiles(&self, percentiles: &[f64]) -> Vec<(f64, Duration)> {
        if self.sample_count == 0 {
            return vec![];
        }

        let mut sorted: Vec<u64> = self.samples[..self.sample_count].to_vec();
        sorted.sort_unstable();

        percentiles
            .iter()
            .filter(|&&p| p >= 0.0 && p <= 100.0 && !p.is_nan())
            .map(|&p| {
                let idx = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
                let idx = idx.min(sorted.len() - 1);
                (p, Duration::from_nanos(sorted[idx]))
            })
            .collect()
    }

    /// Get a snapshot of current metrics.
    #[must_use]
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            total_cycles: self.total_cycles,
            min_ns: if self.total_cycles > 0 {
                Some(self.min_ns)
            } else {
                None
            },
            max_ns: if self.total_cycles > 0 {
                Some(self.max_ns)
            } else {
                None
            },
            mean_ns: if self.total_cycles > 0 {
                Some(self.sum_ns / self.total_cycles)
            } else {
                None
            },
            overrun_count: self.overrun_count,
            sample_count: self.sample_count,
        }
    }

    /// Reset all metrics to initial state.
    pub fn reset(&mut self) {
        self.samples.fill(0);
        self.write_pos = 0;
        self.sample_count = 0;
        self.total_cycles = 0;
        self.min_ns = u64::MAX;
        self.max_ns = 0;
        self.sum_ns = 0;
        self.overrun_count = 0;
    }
}

/// Immutable snapshot of metrics for reporting.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct MetricsSnapshot {
    /// Total cycles executed.
    pub total_cycles: u64,
    /// Minimum cycle time in nanoseconds.
    pub min_ns: Option<u64>,
    /// Maximum cycle time in nanoseconds.
    pub max_ns: Option<u64>,
    /// Mean cycle time in nanoseconds.
    pub mean_ns: Option<u64>,
    /// Number of cycle overruns.
    pub overrun_count: u64,
    /// Number of samples in the histogram.
    pub sample_count: usize,
}

impl MetricsSnapshot {
    /// Get jitter (max - min) in nanoseconds.
    #[must_use]
    pub fn jitter_ns(&self) -> Option<u64> {
        match (self.min_ns, self.max_ns) {
            (Some(min), Some(max)) => Some(max - min),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_recording() {
        let mut metrics = CycleMetrics::new(100, Duration::from_millis(1));

        metrics.record(Duration::from_micros(500));
        metrics.record(Duration::from_micros(600));
        metrics.record(Duration::from_micros(550));

        assert_eq!(metrics.total_cycles(), 3);
        assert_eq!(metrics.min(), Some(Duration::from_micros(500)));
        assert_eq!(metrics.max(), Some(Duration::from_micros(600)));
    }

    #[test]
    fn test_overrun_counting() {
        let mut metrics = CycleMetrics::new(100, Duration::from_millis(1));

        metrics.record(Duration::from_micros(900)); // OK
        metrics.record(Duration::from_micros(1100)); // Overrun
        metrics.record(Duration::from_micros(800)); // OK
        metrics.record(Duration::from_micros(1500)); // Overrun

        assert_eq!(metrics.overrun_count(), 2);
    }

    #[test]
    fn test_percentile_calculation() {
        let mut metrics = CycleMetrics::new(100, Duration::from_millis(1));

        // Record values 1-100 microseconds
        for i in 1..=100 {
            metrics.record(Duration::from_micros(i));
        }

        // p50 should be around 50µs
        let p50 = metrics.percentile(50.0).unwrap();
        assert!(p50.as_micros() >= 49 && p50.as_micros() <= 51);

        // p99 should be around 99µs
        let p99 = metrics.percentile(99.0).unwrap();
        assert!(p99.as_micros() >= 98 && p99.as_micros() <= 100);
    }

    #[test]
    fn test_ring_buffer_wrapping() {
        let mut metrics = CycleMetrics::new(10, Duration::from_millis(1));

        // Fill buffer and wrap around
        for i in 0..25 {
            metrics.record_ns(i * 1000);
        }

        assert_eq!(metrics.total_cycles(), 25);
        // Sample count should be capped at buffer size
        assert_eq!(metrics.snapshot().sample_count, 10);
    }

    #[test]
    fn test_reset() {
        let mut metrics = CycleMetrics::new(100, Duration::from_millis(1));

        metrics.record(Duration::from_micros(500));
        metrics.record(Duration::from_micros(1500)); // Overrun

        metrics.reset();

        assert_eq!(metrics.total_cycles(), 0);
        assert_eq!(metrics.overrun_count(), 0);
        assert!(metrics.min().is_none());
    }

    #[test]
    fn test_snapshot() {
        let mut metrics = CycleMetrics::new(100, Duration::from_millis(1));

        metrics.record(Duration::from_micros(400));
        metrics.record(Duration::from_micros(600));

        let snap = metrics.snapshot();
        assert_eq!(snap.total_cycles, 2);
        assert_eq!(snap.min_ns, Some(400_000));
        assert_eq!(snap.max_ns, Some(600_000));
        assert_eq!(snap.jitter_ns(), Some(200_000));
    }

    #[test]
    fn test_percentile_validation() {
        let mut metrics = CycleMetrics::new(100, Duration::from_millis(1));

        // Record some samples
        for i in 1..=10 {
            metrics.record(Duration::from_micros(i));
        }

        // Valid percentiles should work
        assert!(metrics.percentile(0.0).is_some());
        assert!(metrics.percentile(50.0).is_some());
        assert!(metrics.percentile(100.0).is_some());

        // Invalid percentiles should return None
        assert!(metrics.percentile(-1.0).is_none());
        assert!(metrics.percentile(101.0).is_none());
        assert!(metrics.percentile(f64::NAN).is_none());
        assert!(metrics.percentile(f64::INFINITY).is_none());
        assert!(metrics.percentile(f64::NEG_INFINITY).is_none());
    }

    #[test]
    fn test_percentiles_validation() {
        let mut metrics = CycleMetrics::new(100, Duration::from_millis(1));

        for i in 1..=10 {
            metrics.record(Duration::from_micros(i));
        }

        // Mix of valid and invalid percentiles - only valid ones returned
        let results = metrics.percentiles(&[-10.0, 50.0, 150.0, 99.0, f64::NAN]);
        assert_eq!(results.len(), 2); // Only 50.0 and 99.0 are valid
        assert_eq!(results[0].0, 50.0);
        assert_eq!(results[1].0, 99.0);
    }
}
