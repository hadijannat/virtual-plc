//! Diagnostics and health check module for the PLC daemon.
//!
//! Provides runtime health monitoring, metrics export, and diagnostic
//! information for external monitoring systems (e.g., Prometheus).

use plc_common::metrics::CycleMetrics;
use plc_common::state::RuntimeState;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Health status of the PLC runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    /// System is healthy and operating normally.
    Healthy,
    /// System is degraded but still operational.
    Degraded,
    /// System is unhealthy or in fault state.
    Unhealthy,
    /// System is starting up.
    Starting,
    /// System is shutting down.
    ShuttingDown,
}

impl std::fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HealthStatus::Healthy => write!(f, "healthy"),
            HealthStatus::Degraded => write!(f, "degraded"),
            HealthStatus::Unhealthy => write!(f, "unhealthy"),
            HealthStatus::Starting => write!(f, "starting"),
            HealthStatus::ShuttingDown => write!(f, "shutting_down"),
        }
    }
}

/// Snapshot of runtime diagnostics at a point in time.
#[derive(Debug, Clone)]
pub struct DiagnosticsSnapshot {
    /// Current health status.
    pub health: HealthStatus,
    /// Current runtime state.
    pub state: RuntimeState,
    /// Total cycles executed.
    pub cycle_count: u64,
    /// Number of cycle overruns.
    pub overrun_count: u64,
    /// Uptime since daemon start.
    pub uptime: Duration,
    /// Last cycle execution time.
    pub last_cycle_time: Option<Duration>,
    /// Average cycle time (if available).
    pub avg_cycle_time: Option<Duration>,
    /// Maximum cycle time observed.
    pub max_cycle_time: Option<Duration>,
    /// Whether fieldbus is connected.
    pub fieldbus_connected: bool,
    /// Whether Wasm module is loaded.
    pub wasm_loaded: bool,
}

/// Shared diagnostics state updated by the runtime.
#[derive(Debug)]
pub struct DiagnosticsState {
    /// Total cycles executed.
    cycle_count: AtomicU64,
    /// Number of cycle overruns.
    overrun_count: AtomicU64,
    /// Fieldbus connection status.
    fieldbus_connected: AtomicBool,
    /// Wasm module loaded status.
    wasm_loaded: AtomicBool,
    /// Last cycle time in nanoseconds.
    last_cycle_ns: AtomicU64,
    /// Daemon start time.
    start_time: Instant,
}

impl Default for DiagnosticsState {
    fn default() -> Self {
        Self::new()
    }
}

impl DiagnosticsState {
    /// Create new diagnostics state.
    pub fn new() -> Self {
        Self {
            cycle_count: AtomicU64::new(0),
            overrun_count: AtomicU64::new(0),
            fieldbus_connected: AtomicBool::new(false),
            wasm_loaded: AtomicBool::new(false),
            last_cycle_ns: AtomicU64::new(0),
            start_time: Instant::now(),
        }
    }

    /// Record a completed cycle.
    pub fn record_cycle(&self, execution_time: Duration, overrun: bool) {
        self.cycle_count.fetch_add(1, Ordering::Relaxed);
        self.last_cycle_ns
            .store(execution_time.as_nanos() as u64, Ordering::Relaxed);
        if overrun {
            self.overrun_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Set fieldbus connection status.
    pub fn set_fieldbus_connected(&self, connected: bool) {
        self.fieldbus_connected.store(connected, Ordering::Relaxed);
    }

    /// Set Wasm module loaded status.
    pub fn set_wasm_loaded(&self, loaded: bool) {
        self.wasm_loaded.store(loaded, Ordering::Relaxed);
    }

    /// Get total cycle count.
    pub fn cycle_count(&self) -> u64 {
        self.cycle_count.load(Ordering::Relaxed)
    }

    /// Get overrun count.
    pub fn overrun_count(&self) -> u64 {
        self.overrun_count.load(Ordering::Relaxed)
    }

    /// Get uptime since daemon start.
    pub fn uptime(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Get last cycle time.
    pub fn last_cycle_time(&self) -> Option<Duration> {
        let ns = self.last_cycle_ns.load(Ordering::Relaxed);
        if ns > 0 {
            Some(Duration::from_nanos(ns))
        } else {
            None
        }
    }

    /// Check if fieldbus is connected.
    pub fn is_fieldbus_connected(&self) -> bool {
        self.fieldbus_connected.load(Ordering::Relaxed)
    }

    /// Check if Wasm is loaded.
    pub fn is_wasm_loaded(&self) -> bool {
        self.wasm_loaded.load(Ordering::Relaxed)
    }
}

/// Diagnostics collector that aggregates runtime information.
pub struct DiagnosticsCollector {
    state: Arc<DiagnosticsState>,
}

impl DiagnosticsCollector {
    /// Create a new diagnostics collector.
    pub fn new(state: Arc<DiagnosticsState>) -> Self {
        Self { state }
    }

    /// Determine health status from runtime state.
    pub fn health_from_state(&self, runtime_state: RuntimeState) -> HealthStatus {
        match runtime_state {
            RuntimeState::Boot | RuntimeState::Init | RuntimeState::PreOp => HealthStatus::Starting,
            RuntimeState::Run => {
                // Check for degraded conditions
                let overrun_rate = if self.state.cycle_count() > 0 {
                    self.state.overrun_count() as f64 / self.state.cycle_count() as f64
                } else {
                    0.0
                };

                if overrun_rate > 0.01 {
                    // More than 1% overruns
                    HealthStatus::Degraded
                } else {
                    HealthStatus::Healthy
                }
            }
            RuntimeState::SafeStop => HealthStatus::ShuttingDown,
            RuntimeState::Fault => HealthStatus::Unhealthy,
        }
    }

    /// Create a snapshot of current diagnostics.
    pub fn snapshot(&self, runtime_state: RuntimeState, metrics: &CycleMetrics) -> DiagnosticsSnapshot {
        DiagnosticsSnapshot {
            health: self.health_from_state(runtime_state),
            state: runtime_state,
            cycle_count: self.state.cycle_count(),
            overrun_count: self.state.overrun_count(),
            uptime: self.state.uptime(),
            last_cycle_time: self.state.last_cycle_time(),
            avg_cycle_time: metrics.mean(),
            max_cycle_time: metrics.max(),
            fieldbus_connected: self.state.is_fieldbus_connected(),
            wasm_loaded: self.state.is_wasm_loaded(),
        }
    }

    /// Get the underlying state for updates.
    pub fn state(&self) -> &Arc<DiagnosticsState> {
        &self.state
    }
}

/// Format metrics for Prometheus text exposition format.
pub fn format_prometheus_metrics(snapshot: &DiagnosticsSnapshot, target_cycle_ns: u64) -> String {
    let mut output = String::new();

    // Health status (1 = healthy, 0 = not)
    output.push_str("# HELP plc_health PLC health status (1=healthy, 0=not healthy)\n");
    output.push_str("# TYPE plc_health gauge\n");
    output.push_str(&format!(
        "plc_health {{status=\"{}\"}} {}\n",
        snapshot.health,
        if snapshot.health == HealthStatus::Healthy {
            1
        } else {
            0
        }
    ));

    // Runtime state
    output.push_str("# HELP plc_state Current runtime state\n");
    output.push_str("# TYPE plc_state gauge\n");
    output.push_str(&format!(
        "plc_state {{state=\"{}\"}} 1\n",
        snapshot.state
    ));

    // Cycle count
    output.push_str("# HELP plc_cycles_total Total PLC cycles executed\n");
    output.push_str("# TYPE plc_cycles_total counter\n");
    output.push_str(&format!("plc_cycles_total {}\n", snapshot.cycle_count));

    // Overrun count
    output.push_str("# HELP plc_overruns_total Total cycle overruns\n");
    output.push_str("# TYPE plc_overruns_total counter\n");
    output.push_str(&format!("plc_overruns_total {}\n", snapshot.overrun_count));

    // Uptime
    output.push_str("# HELP plc_uptime_seconds Daemon uptime in seconds\n");
    output.push_str("# TYPE plc_uptime_seconds gauge\n");
    output.push_str(&format!(
        "plc_uptime_seconds {:.3}\n",
        snapshot.uptime.as_secs_f64()
    ));

    // Cycle time metrics
    if let Some(last) = snapshot.last_cycle_time {
        output.push_str("# HELP plc_cycle_time_seconds Last cycle execution time\n");
        output.push_str("# TYPE plc_cycle_time_seconds gauge\n");
        output.push_str(&format!(
            "plc_cycle_time_seconds {:.9}\n",
            last.as_secs_f64()
        ));
    }

    if let Some(avg) = snapshot.avg_cycle_time {
        output.push_str("# HELP plc_cycle_time_avg_seconds Average cycle execution time\n");
        output.push_str("# TYPE plc_cycle_time_avg_seconds gauge\n");
        output.push_str(&format!(
            "plc_cycle_time_avg_seconds {:.9}\n",
            avg.as_secs_f64()
        ));
    }

    if let Some(max) = snapshot.max_cycle_time {
        output.push_str("# HELP plc_cycle_time_max_seconds Maximum cycle execution time\n");
        output.push_str("# TYPE plc_cycle_time_max_seconds gauge\n");
        output.push_str(&format!(
            "plc_cycle_time_max_seconds {:.9}\n",
            max.as_secs_f64()
        ));
    }

    // Target cycle time
    output.push_str("# HELP plc_cycle_time_target_seconds Target cycle time\n");
    output.push_str("# TYPE plc_cycle_time_target_seconds gauge\n");
    output.push_str(&format!(
        "plc_cycle_time_target_seconds {:.9}\n",
        Duration::from_nanos(target_cycle_ns).as_secs_f64()
    ));

    // Connection status
    output.push_str("# HELP plc_fieldbus_connected Fieldbus connection status\n");
    output.push_str("# TYPE plc_fieldbus_connected gauge\n");
    output.push_str(&format!(
        "plc_fieldbus_connected {}\n",
        if snapshot.fieldbus_connected { 1 } else { 0 }
    ));

    output.push_str("# HELP plc_wasm_loaded Wasm module loaded status\n");
    output.push_str("# TYPE plc_wasm_loaded gauge\n");
    output.push_str(&format!(
        "plc_wasm_loaded {}\n",
        if snapshot.wasm_loaded { 1 } else { 0 }
    ));

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diagnostics_state_new() {
        let state = DiagnosticsState::new();
        assert_eq!(state.cycle_count(), 0);
        assert_eq!(state.overrun_count(), 0);
        assert!(!state.is_fieldbus_connected());
        assert!(!state.is_wasm_loaded());
        assert!(state.last_cycle_time().is_none());
    }

    #[test]
    fn test_record_cycle() {
        let state = DiagnosticsState::new();
        state.record_cycle(Duration::from_micros(500), false);
        assert_eq!(state.cycle_count(), 1);
        assert_eq!(state.overrun_count(), 0);
        assert_eq!(state.last_cycle_time(), Some(Duration::from_micros(500)));

        state.record_cycle(Duration::from_micros(1200), true);
        assert_eq!(state.cycle_count(), 2);
        assert_eq!(state.overrun_count(), 1);
    }

    #[test]
    fn test_health_status_display() {
        assert_eq!(format!("{}", HealthStatus::Healthy), "healthy");
        assert_eq!(format!("{}", HealthStatus::Degraded), "degraded");
        assert_eq!(format!("{}", HealthStatus::Unhealthy), "unhealthy");
    }

    #[test]
    fn test_health_from_state() {
        let state = Arc::new(DiagnosticsState::new());
        let collector = DiagnosticsCollector::new(state);

        assert_eq!(
            collector.health_from_state(RuntimeState::Boot),
            HealthStatus::Starting
        );
        assert_eq!(
            collector.health_from_state(RuntimeState::Run),
            HealthStatus::Healthy
        );
        assert_eq!(
            collector.health_from_state(RuntimeState::Fault),
            HealthStatus::Unhealthy
        );
        assert_eq!(
            collector.health_from_state(RuntimeState::SafeStop),
            HealthStatus::ShuttingDown
        );
    }

    #[test]
    fn test_degraded_health_on_overruns() {
        let state = Arc::new(DiagnosticsState::new());
        let collector = DiagnosticsCollector::new(Arc::clone(&state));

        // Simulate 2% overrun rate
        for i in 0..100 {
            state.record_cycle(Duration::from_micros(500), i < 2);
        }

        assert_eq!(
            collector.health_from_state(RuntimeState::Run),
            HealthStatus::Degraded
        );
    }

    #[test]
    fn test_prometheus_metrics_format() {
        let snapshot = DiagnosticsSnapshot {
            health: HealthStatus::Healthy,
            state: RuntimeState::Run,
            cycle_count: 1000,
            overrun_count: 5,
            uptime: Duration::from_secs(3600),
            last_cycle_time: Some(Duration::from_micros(800)),
            avg_cycle_time: Some(Duration::from_micros(750)),
            max_cycle_time: Some(Duration::from_micros(1200)),
            fieldbus_connected: true,
            wasm_loaded: true,
        };

        let output = format_prometheus_metrics(&snapshot, 1_000_000);

        assert!(output.contains("plc_health"));
        assert!(output.contains("plc_cycles_total 1000"));
        assert!(output.contains("plc_overruns_total 5"));
        assert!(output.contains("plc_fieldbus_connected 1"));
        assert!(output.contains("plc_wasm_loaded 1"));
    }
}
