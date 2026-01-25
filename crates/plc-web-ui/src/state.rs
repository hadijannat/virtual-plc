//! Shared state for web UI.
//!
//! This module defines the shared state that is updated by the runtime
//! and read by the web API and WebSocket handlers.

use crate::PlcMetrics;
use plc_common::state::RuntimeState;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use std::time::Instant;
use tokio::sync::broadcast;

/// I/O state snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IoSnapshot {
    /// Digital inputs (bit-packed).
    pub digital_inputs: u32,
    /// Digital outputs (bit-packed).
    pub digital_outputs: u32,
    /// Analog inputs (16 channels).
    pub analog_inputs: Vec<i16>,
    /// Analog outputs (16 channels).
    pub analog_outputs: Vec<i16>,
}

/// Cycle metrics snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    /// Total cycles executed.
    pub total_cycles: u64,
    /// Minimum cycle time in microseconds.
    pub min_us: u64,
    /// Maximum cycle time in microseconds.
    pub max_us: u64,
    /// Average cycle time in microseconds.
    pub avg_us: u64,
    /// Configured cycle time in microseconds.
    pub target_us: u64,
    /// Number of overruns.
    pub overrun_count: u64,
    /// Jitter (max - min) in microseconds.
    pub jitter_us: u64,
}

/// Fault record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaultRecord {
    /// Cycle number when fault occurred.
    pub cycle: u64,
    /// Fault reason.
    pub reason: String,
    /// Timestamp (relative to session start).
    pub timestamp_ms: u64,
}

/// Complete state snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StateSnapshot {
    /// Runtime state (Boot, Init, Run, etc.).
    pub runtime_state: String,
    /// I/O values.
    pub io: IoSnapshot,
    /// Cycle metrics.
    pub metrics: MetricsSnapshot,
    /// Recent faults (last N).
    pub faults: Vec<FaultRecord>,
    /// Timestamp of this snapshot (ms since epoch).
    pub timestamp_ms: u64,
}

/// State update message for WebSocket broadcast.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StateUpdate {
    /// Full state snapshot.
    #[serde(rename = "full")]
    Full(StateSnapshot),
    /// Incremental I/O update.
    #[serde(rename = "io")]
    Io(IoSnapshot),
    /// Metrics update.
    #[serde(rename = "metrics")]
    Metrics(MetricsSnapshot),
    /// New fault.
    #[serde(rename = "fault")]
    Fault(FaultRecord),
    /// Runtime state change.
    #[serde(rename = "state")]
    StateChange { state: String },
}

/// Shared state container.
#[derive(Debug, Default)]
pub struct SharedState {
    /// Current runtime state.
    pub runtime_state: RwLock<RuntimeState>,
    /// Current I/O snapshot.
    pub io: RwLock<IoSnapshot>,
    /// Current metrics snapshot.
    pub metrics: RwLock<MetricsSnapshot>,
    /// Recent faults.
    pub faults: RwLock<Vec<FaultRecord>>,
    /// Session start time.
    pub session_start: RwLock<Option<Instant>>,
}

impl SharedState {
    /// Get a full state snapshot.
    pub fn snapshot(&self) -> StateSnapshot {
        let runtime_state = self
            .runtime_state
            .read()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let io = self.io.read().map(|i| i.clone()).unwrap_or_default();
        let metrics = self.metrics.read().map(|m| m.clone()).unwrap_or_default();
        let faults = self.faults.read().map(|f| f.clone()).unwrap_or_default();

        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        StateSnapshot {
            runtime_state,
            io,
            metrics,
            faults,
            timestamp_ms,
        }
    }
}

/// Handle for updating shared state from the runtime.
#[derive(Clone)]
pub struct StateUpdater {
    pub(crate) state: Arc<SharedState>,
    pub(crate) broadcast_tx: broadcast::Sender<StateUpdate>,
    pub(crate) metrics: Option<Arc<PlcMetrics>>,
}

impl StateUpdater {
    fn runtime_state_metric_value(state: RuntimeState) -> i64 {
        match state {
            RuntimeState::Boot => 0,
            RuntimeState::Init => 1,
            RuntimeState::PreOp => 2,
            RuntimeState::Run => 3,
            RuntimeState::Fault => 4,
            RuntimeState::SafeStop => 5,
        }
    }

    /// Update the runtime state.
    pub fn set_runtime_state(&self, state: RuntimeState) {
        if let Ok(mut guard) = self.state.runtime_state.write() {
            *guard = state;
        }
        let _ = self.broadcast_tx.send(StateUpdate::StateChange {
            state: state.to_string(),
        });

        // Update Prometheus metrics
        if let Some(ref metrics) = self.metrics {
            metrics
                .runtime_state
                .set(Self::runtime_state_metric_value(state));
        }
    }

    /// Update I/O values.
    pub fn update_io(&self, io: IoSnapshot) {
        if let Ok(mut guard) = self.state.io.write() {
            *guard = io.clone();
        }
        let _ = self.broadcast_tx.send(StateUpdate::Io(io.clone()));

        // Update Prometheus metrics
        if let Some(ref metrics) = self.metrics {
            metrics.update_from_io(&io);
        }
    }

    /// Update metrics.
    pub fn update_metrics(&self, metrics: MetricsSnapshot) {
        if let Ok(mut guard) = self.state.metrics.write() {
            *guard = metrics.clone();
        }
        let _ = self
            .broadcast_tx
            .send(StateUpdate::Metrics(metrics.clone()));

        // Update Prometheus metrics
        if let Some(ref prom_metrics) = self.metrics {
            prom_metrics.update_from_snapshot(&metrics);
        }
    }

    /// Record a fault.
    pub fn record_fault(&self, reason: String, cycle: u64) {
        let timestamp_ms = self
            .state
            .session_start
            .read()
            .ok()
            .and_then(|start| start.as_ref().map(|s| s.elapsed().as_millis() as u64))
            .unwrap_or(0);

        let fault = FaultRecord {
            cycle,
            reason,
            timestamp_ms,
        };

        if let Ok(mut guard) = self.state.faults.write() {
            guard.push(fault.clone());
            // Keep only last 100 faults
            if guard.len() > 100 {
                guard.remove(0);
            }
        }
        let _ = self.broadcast_tx.send(StateUpdate::Fault(fault));

        // Update Prometheus metrics
        if let Some(ref metrics) = self.metrics {
            metrics.record_fault();
        }
    }

    /// Send a full state snapshot (useful for new WebSocket connections).
    pub fn broadcast_full_state(&self) {
        let snapshot = self.state.snapshot();
        let _ = self.broadcast_tx.send(StateUpdate::Full(snapshot));
    }

    /// Mark session start time.
    pub fn start_session(&self) {
        if let Ok(mut guard) = self.state.session_start.write() {
            *guard = Some(Instant::now());
        }
    }

    /// Update I/O from raw values (convenience method).
    pub fn update_io_raw(
        &self,
        digital_inputs: u32,
        digital_outputs: u32,
        analog_inputs: &[i16],
        analog_outputs: &[i16],
    ) {
        let io = IoSnapshot {
            digital_inputs,
            digital_outputs,
            analog_inputs: analog_inputs.to_vec(),
            analog_outputs: analog_outputs.to_vec(),
        };
        self.update_io(io);
    }

    /// Update metrics from raw values (convenience method).
    pub fn update_metrics_raw(
        &self,
        total_cycles: u64,
        min_ns: u64,
        max_ns: u64,
        avg_ns: u64,
        target_ns: u64,
        overrun_count: u64,
    ) {
        let metrics = MetricsSnapshot {
            total_cycles,
            min_us: min_ns / 1000,
            max_us: max_ns / 1000,
            avg_us: avg_ns / 1000,
            target_us: target_ns / 1000,
            overrun_count,
            jitter_us: (max_ns.saturating_sub(min_ns)) / 1000,
        };
        self.update_metrics(metrics);
    }
}
