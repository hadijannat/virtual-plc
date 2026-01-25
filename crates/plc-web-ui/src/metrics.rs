//! Prometheus metrics for PLC runtime monitoring.
//!
//! Exposes key runtime metrics in Prometheus text format at `/metrics`.

use axum::{
    http::{header::CONTENT_TYPE, StatusCode},
    response::IntoResponse,
};
use prometheus::{
    Gauge, GaugeVec, Histogram, HistogramOpts, IntCounter, IntGauge, Opts, Registry, TextEncoder,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Prometheus metrics registry and collectors.
pub struct PlcMetrics {
    /// The registry holding all metrics.
    registry: Registry,

    /// Total cycles executed.
    pub cycles_total: IntCounter,

    /// Current runtime state (0=Boot, 1=PreOp, 2=Run, 3=Stop, 4=Fault).
    pub runtime_state: IntGauge,

    /// Number of cycle overruns.
    pub overruns_total: IntCounter,

    /// Current cycle time in microseconds.
    pub cycle_time_us: Gauge,

    /// Minimum cycle time in microseconds.
    pub cycle_time_min_us: Gauge,

    /// Maximum cycle time in microseconds.
    pub cycle_time_max_us: Gauge,

    /// Average cycle time in microseconds.
    pub cycle_time_avg_us: Gauge,

    /// Cycle time jitter (max - min) in microseconds.
    pub cycle_jitter_us: Gauge,

    /// Target cycle time in microseconds.
    pub cycle_target_us: Gauge,

    /// Histogram of cycle execution times.
    pub cycle_duration: Histogram,

    /// Number of connected WebSocket clients.
    pub websocket_clients: IntGauge,

    /// Total faults recorded.
    pub faults_total: IntCounter,

    /// Digital inputs state (bit-packed as gauge).
    pub digital_inputs: Gauge,

    /// Digital outputs state (bit-packed as gauge).
    pub digital_outputs: Gauge,

    /// Analog inputs by channel.
    pub analog_inputs: GaugeVec,

    /// Analog outputs by channel.
    pub analog_outputs: GaugeVec,

    /// Last observed total cycles (for counter synchronization).
    last_cycles_total: AtomicU64,

    /// Last observed total overruns (for counter synchronization).
    last_overruns_total: AtomicU64,
}

impl PlcMetrics {
    /// Create a new metrics instance with a custom registry.
    pub fn new() -> Self {
        let registry = Registry::new();

        let cycles_total = IntCounter::new(
            "plc_cycles_total",
            "Total number of PLC scan cycles executed",
        )
        .expect("metric creation should succeed");

        let runtime_state = IntGauge::new(
            "plc_runtime_state",
            "Current runtime state (0=Boot, 1=Init, 2=PreOp, 3=Run, 4=Fault, 5=SafeStop)",
        )
        .expect("metric creation should succeed");

        let overruns_total =
            IntCounter::new("plc_overruns_total", "Total number of cycle time overruns")
                .expect("metric creation should succeed");

        let cycle_time_us = Gauge::new(
            "plc_cycle_time_microseconds",
            "Current cycle execution time in microseconds",
        )
        .expect("metric creation should succeed");

        let cycle_time_min_us = Gauge::new(
            "plc_cycle_time_min_microseconds",
            "Minimum cycle execution time in microseconds",
        )
        .expect("metric creation should succeed");

        let cycle_time_max_us = Gauge::new(
            "plc_cycle_time_max_microseconds",
            "Maximum cycle execution time in microseconds",
        )
        .expect("metric creation should succeed");

        let cycle_time_avg_us = Gauge::new(
            "plc_cycle_time_avg_microseconds",
            "Average cycle execution time in microseconds",
        )
        .expect("metric creation should succeed");

        let cycle_jitter_us = Gauge::new(
            "plc_cycle_jitter_microseconds",
            "Cycle time jitter (max - min) in microseconds",
        )
        .expect("metric creation should succeed");

        let cycle_target_us = Gauge::new(
            "plc_cycle_target_microseconds",
            "Configured target cycle time in microseconds",
        )
        .expect("metric creation should succeed");

        let cycle_duration = Histogram::with_opts(
            HistogramOpts::new(
                "plc_cycle_duration_seconds",
                "Histogram of cycle execution times in seconds",
            )
            .buckets(vec![
                0.000_01, // 10 us
                0.000_05, // 50 us
                0.000_1,  // 100 us
                0.000_25, // 250 us
                0.000_5,  // 500 us
                0.001,    // 1 ms
                0.002_5,  // 2.5 ms
                0.005,    // 5 ms
                0.01,     // 10 ms
                0.025,    // 25 ms
                0.05,     // 50 ms
                0.1,      // 100 ms
            ]),
        )
        .expect("metric creation should succeed");

        let websocket_clients = IntGauge::new(
            "plc_websocket_clients",
            "Number of connected WebSocket clients",
        )
        .expect("metric creation should succeed");

        let faults_total = IntCounter::new("plc_faults_total", "Total number of faults recorded")
            .expect("metric creation should succeed");

        let digital_inputs = Gauge::new(
            "plc_digital_inputs",
            "Digital inputs state (bit-packed as integer)",
        )
        .expect("metric creation should succeed");

        let digital_outputs = Gauge::new(
            "plc_digital_outputs",
            "Digital outputs state (bit-packed as integer)",
        )
        .expect("metric creation should succeed");

        let analog_inputs = GaugeVec::new(
            Opts::new("plc_analog_input", "Analog input value by channel"),
            &["channel"],
        )
        .expect("metric creation should succeed");

        let analog_outputs = GaugeVec::new(
            Opts::new("plc_analog_output", "Analog output value by channel"),
            &["channel"],
        )
        .expect("metric creation should succeed");

        // Register all metrics
        registry
            .register(Box::new(cycles_total.clone()))
            .expect("registration should succeed");
        registry
            .register(Box::new(runtime_state.clone()))
            .expect("registration should succeed");
        registry
            .register(Box::new(overruns_total.clone()))
            .expect("registration should succeed");
        registry
            .register(Box::new(cycle_time_us.clone()))
            .expect("registration should succeed");
        registry
            .register(Box::new(cycle_time_min_us.clone()))
            .expect("registration should succeed");
        registry
            .register(Box::new(cycle_time_max_us.clone()))
            .expect("registration should succeed");
        registry
            .register(Box::new(cycle_time_avg_us.clone()))
            .expect("registration should succeed");
        registry
            .register(Box::new(cycle_jitter_us.clone()))
            .expect("registration should succeed");
        registry
            .register(Box::new(cycle_target_us.clone()))
            .expect("registration should succeed");
        registry
            .register(Box::new(cycle_duration.clone()))
            .expect("registration should succeed");
        registry
            .register(Box::new(websocket_clients.clone()))
            .expect("registration should succeed");
        registry
            .register(Box::new(faults_total.clone()))
            .expect("registration should succeed");
        registry
            .register(Box::new(digital_inputs.clone()))
            .expect("registration should succeed");
        registry
            .register(Box::new(digital_outputs.clone()))
            .expect("registration should succeed");
        registry
            .register(Box::new(analog_inputs.clone()))
            .expect("registration should succeed");
        registry
            .register(Box::new(analog_outputs.clone()))
            .expect("registration should succeed");

        Self {
            registry,
            cycles_total,
            runtime_state,
            overruns_total,
            cycle_time_us,
            cycle_time_min_us,
            cycle_time_max_us,
            cycle_time_avg_us,
            cycle_jitter_us,
            cycle_target_us,
            cycle_duration,
            websocket_clients,
            faults_total,
            digital_inputs,
            digital_outputs,
            analog_inputs,
            analog_outputs,
            last_cycles_total: AtomicU64::new(0),
            last_overruns_total: AtomicU64::new(0),
        }
    }

    /// Record a completed cycle with its execution time.
    pub fn record_cycle(&self, duration_us: u64) {
        self.cycles_total.inc();
        self.last_cycles_total.fetch_add(1, Ordering::Relaxed);
        self.cycle_time_us.set(duration_us as f64);
        self.cycle_duration
            .observe(duration_us as f64 / 1_000_000.0);
    }

    /// Record a cycle overrun.
    pub fn record_overrun(&self) {
        self.overruns_total.inc();
        self.last_overruns_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a fault.
    pub fn record_fault(&self) {
        self.faults_total.inc();
    }

    /// Update metrics from a metrics snapshot.
    pub fn update_from_snapshot(&self, metrics: &crate::state::MetricsSnapshot) {
        let new_cycles = metrics.total_cycles;
        let last_cycles = self.last_cycles_total.load(Ordering::Relaxed);
        if new_cycles < last_cycles {
            self.cycles_total.reset();
            if new_cycles > 0 {
                self.cycles_total.inc_by(new_cycles);
            }
        } else {
            let delta = new_cycles - last_cycles;
            if delta > 0 {
                self.cycles_total.inc_by(delta);
            }
        }
        self.last_cycles_total.store(new_cycles, Ordering::Relaxed);

        let new_overruns = metrics.overrun_count;
        let last_overruns = self.last_overruns_total.load(Ordering::Relaxed);
        if new_overruns < last_overruns {
            self.overruns_total.reset();
            if new_overruns > 0 {
                self.overruns_total.inc_by(new_overruns);
            }
        } else {
            let delta = new_overruns - last_overruns;
            if delta > 0 {
                self.overruns_total.inc_by(delta);
            }
        }
        self.last_overruns_total
            .store(new_overruns, Ordering::Relaxed);

        self.cycle_time_min_us.set(metrics.min_us as f64);
        self.cycle_time_max_us.set(metrics.max_us as f64);
        self.cycle_time_avg_us.set(metrics.avg_us as f64);
        self.cycle_jitter_us.set(metrics.jitter_us as f64);
        self.cycle_target_us.set(metrics.target_us as f64);
    }

    /// Update I/O metrics from an I/O snapshot.
    pub fn update_from_io(&self, io: &crate::state::IoSnapshot) {
        self.digital_inputs.set(io.digital_inputs as f64);
        self.digital_outputs.set(io.digital_outputs as f64);

        for (i, &value) in io.analog_inputs.iter().enumerate() {
            self.analog_inputs
                .with_label_values(&[&i.to_string()])
                .set(f64::from(value));
        }

        for (i, &value) in io.analog_outputs.iter().enumerate() {
            self.analog_outputs
                .with_label_values(&[&i.to_string()])
                .set(f64::from(value));
        }
    }

    /// Render metrics in Prometheus text format.
    pub fn render(&self) -> Result<String, prometheus::Error> {
        let encoder = TextEncoder::new();
        let metric_families = self.registry.gather();
        encoder.encode_to_string(&metric_families)
    }
}

impl Default for PlcMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Metrics endpoint handler.
///
/// GET /metrics
pub async fn metrics_handler(
    axum::extract::Extension(metrics): axum::extract::Extension<Arc<PlcMetrics>>,
) -> impl IntoResponse {
    match metrics.render() {
        Ok(output) => (
            StatusCode::OK,
            [(CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
            output,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to render metrics: {}", e),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_creation() {
        let metrics = PlcMetrics::new();
        assert_eq!(metrics.cycles_total.get(), 0);
        assert_eq!(metrics.runtime_state.get(), 0);
    }

    #[test]
    fn test_record_cycle() {
        let metrics = PlcMetrics::new();
        metrics.record_cycle(500);
        assert_eq!(metrics.cycles_total.get(), 1);
        assert!((metrics.cycle_time_us.get() - 500.0).abs() < 0.001);
    }

    #[test]
    fn test_record_overrun() {
        let metrics = PlcMetrics::new();
        metrics.record_overrun();
        metrics.record_overrun();
        assert_eq!(metrics.overruns_total.get(), 2);
    }

    #[test]
    fn test_render() {
        let metrics = PlcMetrics::new();
        metrics.record_cycle(100);

        let output = metrics.render().expect("should render");
        assert!(output.contains("plc_cycles_total"));
        assert!(output.contains("plc_cycle_time_microseconds"));
    }

    #[test]
    fn test_io_update() {
        let metrics = PlcMetrics::new();
        let io = crate::state::IoSnapshot {
            digital_inputs: 0xFF,
            digital_outputs: 0x0F,
            analog_inputs: vec![100, 200, 300],
            analog_outputs: vec![400, 500],
        };

        metrics.update_from_io(&io);

        assert!((metrics.digital_inputs.get() - 255.0).abs() < 0.001);
        assert!((metrics.digital_outputs.get() - 15.0).abs() < 0.001);
    }

    #[test]
    fn test_update_from_snapshot_updates_totals() {
        let metrics = PlcMetrics::new();

        let snapshot = crate::state::MetricsSnapshot {
            total_cycles: 10,
            min_us: 100,
            max_us: 200,
            avg_us: 150,
            target_us: 100,
            overrun_count: 2,
            jitter_us: 100,
        };
        metrics.update_from_snapshot(&snapshot);
        assert_eq!(metrics.cycles_total.get(), 10);
        assert_eq!(metrics.overruns_total.get(), 2);

        let snapshot = crate::state::MetricsSnapshot {
            total_cycles: 15,
            min_us: 100,
            max_us: 200,
            avg_us: 150,
            target_us: 100,
            overrun_count: 3,
            jitter_us: 100,
        };
        metrics.update_from_snapshot(&snapshot);
        assert_eq!(metrics.cycles_total.get(), 15);
        assert_eq!(metrics.overruns_total.get(), 3);
    }
}
