//! Fault frame recording for postmortem diagnosis.
//!
//! This module provides a pre-allocated ring buffer that captures system state
//! at each cycle. When a fault occurs, the last N frames are available for
//! analysis, enabling root cause identification without runtime memory allocation.
//!
//! # Design
//!
//! - **Pre-allocated**: All memory is allocated upfront to avoid heap activity in RT path.
//! - **Fixed-size buffers**: Input/output snapshots use fixed arrays, not Vec.
//! - **Lock-free recording**: Single-threaded recording path has no synchronization overhead.
//! - **Configurable depth**: Default 64 frames, adjustable for memory vs. history tradeoff.

use crate::io_image::ProcessData;
use crate::scheduler::CyclePhaseTimings;
use static_assertions::const_assert;
use std::time::Duration;

/// Default number of fault frames to retain.
pub const DEFAULT_FAULT_FRAME_COUNT: usize = 64;

/// Maximum size of I/O snapshot in bytes.
/// Sized to hold ProcessData's digital and analog I/O.
pub const IO_SNAPSHOT_SIZE: usize = 256;

// Compile-time check that IO_SNAPSHOT_SIZE can hold ProcessData's I/O fields.
// ProcessData has: digital_inputs[1] + digital_outputs[1] (8 bytes) +
//                  analog_inputs[16] + analog_outputs[16] (64 bytes) = 72 bytes minimum
// We use 256 to allow for future expansion.
const_assert!(IO_SNAPSHOT_SIZE >= std::mem::size_of::<ProcessData>());

/// Reason for entering fault state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultReason {
    /// No fault (normal frame capture).
    None,
    /// Cycle execution exceeded deadline.
    CycleOverrun,
    /// WebAssembly execution trapped.
    WasmTrap,
    /// Watchdog timer expired.
    WatchdogTimeout,
    /// Fieldbus communication failure.
    FieldbusError,
    /// Working counter mismatch (EtherCAT).
    WkcError,
    /// Logic engine returned error.
    LogicError,
    /// External fault trigger.
    External,
}

impl Default for FaultReason {
    fn default() -> Self {
        Self::None
    }
}

impl std::fmt::Display for FaultReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "NONE"),
            Self::CycleOverrun => write!(f, "CYCLE_OVERRUN"),
            Self::WasmTrap => write!(f, "WASM_TRAP"),
            Self::WatchdogTimeout => write!(f, "WATCHDOG_TIMEOUT"),
            Self::FieldbusError => write!(f, "FIELDBUS_ERROR"),
            Self::WkcError => write!(f, "WKC_ERROR"),
            Self::LogicError => write!(f, "LOGIC_ERROR"),
            Self::External => write!(f, "EXTERNAL"),
        }
    }
}

/// A single frame of fault recorder data.
///
/// Captures the complete state at one PLC cycle for postmortem analysis.
#[derive(Debug, Clone)]
pub struct FaultFrame {
    /// Cycle number when this frame was captured.
    pub cycle: u64,
    /// Timestamp in nanoseconds since recorder start.
    pub timestamp_ns: u64,
    /// Snapshot of input data.
    pub inputs: [u8; IO_SNAPSHOT_SIZE],
    /// Snapshot of output data.
    pub outputs: [u8; IO_SNAPSHOT_SIZE],
    /// Per-phase timing breakdown.
    pub phase_timings: CyclePhaseTimings,
    /// Working counter from fieldbus (if applicable).
    pub wkc: Option<u16>,
    /// Expected working counter.
    pub expected_wkc: Option<u16>,
    /// Fault reason (None for normal frames).
    pub fault_reason: FaultReason,
    /// Whether this frame has valid data.
    pub valid: bool,
}

impl Default for FaultFrame {
    fn default() -> Self {
        Self {
            cycle: 0,
            timestamp_ns: 0,
            inputs: [0; IO_SNAPSHOT_SIZE],
            outputs: [0; IO_SNAPSHOT_SIZE],
            phase_timings: CyclePhaseTimings::default(),
            wkc: None,
            expected_wkc: None,
            fault_reason: FaultReason::None,
            valid: false,
        }
    }
}

impl FaultFrame {
    /// Create a new fault frame with cycle data.
    pub fn new(cycle: u64, timestamp_ns: u64, phase_timings: CyclePhaseTimings) -> Self {
        Self {
            cycle,
            timestamp_ns,
            phase_timings,
            valid: true,
            ..Default::default()
        }
    }

    /// Set input snapshot from ProcessData.
    pub fn set_inputs(&mut self, data: &ProcessData) {
        // Pack digital inputs (4 bytes)
        self.inputs[0..4].copy_from_slice(&data.digital_inputs[0].to_le_bytes());
        // Pack analog inputs (32 bytes = 16 x 2 bytes)
        for (i, &val) in data.analog_inputs.iter().enumerate() {
            let offset = 4 + i * 2;
            if offset + 2 <= self.inputs.len() {
                self.inputs[offset..offset + 2].copy_from_slice(&val.to_le_bytes());
            }
        }
    }

    /// Set output snapshot from ProcessData.
    pub fn set_outputs(&mut self, data: &ProcessData) {
        // Pack digital outputs (4 bytes)
        self.outputs[0..4].copy_from_slice(&data.digital_outputs[0].to_le_bytes());
        // Pack analog outputs (32 bytes = 16 x 2 bytes)
        for (i, &val) in data.analog_outputs.iter().enumerate() {
            let offset = 4 + i * 2;
            if offset + 2 <= self.outputs.len() {
                self.outputs[offset..offset + 2].copy_from_slice(&val.to_le_bytes());
            }
        }
    }

    /// Set WKC values.
    pub fn set_wkc(&mut self, actual: u16, expected: u16) {
        self.wkc = Some(actual);
        self.expected_wkc = Some(expected);
    }

    /// Mark this frame as a fault frame with the given reason.
    pub fn set_fault(&mut self, reason: FaultReason) {
        self.fault_reason = reason;
    }
}

/// Pre-allocated ring buffer for fault frame recording.
///
/// Captures system state at each cycle for postmortem diagnosis.
/// When a fault occurs, call `freeze()` to prevent further recording
/// and preserve the fault context.
#[derive(Debug)]
pub struct FaultRecorder {
    /// Ring buffer of fault frames.
    frames: Box<[FaultFrame]>,
    /// Current write position.
    write_pos: usize,
    /// Number of frames written (saturates at capacity).
    frame_count: usize,
    /// Start timestamp for relative timing.
    start_time: std::time::Instant,
    /// Whether recording is frozen (after fault).
    frozen: bool,
    /// Index of the fault frame (if any).
    fault_frame_index: Option<usize>,
}

impl FaultRecorder {
    /// Create a new fault recorder with the specified capacity.
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        let frames: Vec<FaultFrame> = (0..capacity).map(|_| FaultFrame::default()).collect();

        Self {
            frames: frames.into_boxed_slice(),
            write_pos: 0,
            frame_count: 0,
            start_time: std::time::Instant::now(),
            frozen: false,
            fault_frame_index: None,
        }
    }

    /// Create a new fault recorder with default capacity.
    pub fn with_default_capacity() -> Self {
        Self::new(DEFAULT_FAULT_FRAME_COUNT)
    }

    /// Get the capacity (maximum number of frames).
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.frames.len()
    }

    /// Get the number of valid frames recorded.
    #[must_use]
    pub fn frame_count(&self) -> usize {
        self.frame_count.min(self.frames.len())
    }

    /// Check if recording is frozen.
    #[must_use]
    pub fn is_frozen(&self) -> bool {
        self.frozen
    }

    /// Record a normal cycle frame.
    ///
    /// Returns the frame reference for additional data population.
    /// Returns `None` if the recorder is frozen.
    pub fn record_cycle(
        &mut self,
        cycle: u64,
        phase_timings: CyclePhaseTimings,
    ) -> Option<&mut FaultFrame> {
        if self.frozen {
            return None;
        }

        let timestamp_ns = self.start_time.elapsed().as_nanos() as u64;
        let idx = self.write_pos;

        self.frames[idx] = FaultFrame::new(cycle, timestamp_ns, phase_timings);

        self.write_pos = (self.write_pos + 1) % self.frames.len();
        self.frame_count = self.frame_count.saturating_add(1);

        Some(&mut self.frames[idx])
    }

    /// Record a fault and freeze the recorder.
    ///
    /// Creates a NEW dedicated fault frame with the given cycle data,
    /// ensuring the fault is recorded with the correct cycle number and timing.
    /// This prevents the issue where a fault would be attributed to the
    /// previous cycle's frame.
    pub fn record_fault(
        &mut self,
        cycle: u64,
        reason: FaultReason,
        phase_timings: CyclePhaseTimings,
    ) {
        if self.frozen {
            return;
        }

        // Create a dedicated fault frame with current cycle data
        let timestamp_ns = self.start_time.elapsed().as_nanos() as u64;
        let idx = self.write_pos;

        self.frames[idx] = FaultFrame::new(cycle, timestamp_ns, phase_timings);
        self.frames[idx].set_fault(reason);
        self.fault_frame_index = Some(idx);

        self.write_pos = (self.write_pos + 1) % self.frames.len();
        self.frame_count = self.frame_count.saturating_add(1);
        self.frozen = true;
    }

    /// Record a fault with I/O data and freeze the recorder.
    ///
    /// Like `record_fault`, but also captures input and output snapshots
    /// for complete postmortem analysis.
    pub fn record_fault_with_io(
        &mut self,
        cycle: u64,
        reason: FaultReason,
        phase_timings: CyclePhaseTimings,
        inputs: &ProcessData,
        outputs: &ProcessData,
    ) {
        if self.frozen {
            return;
        }

        // Create a dedicated fault frame with current cycle data
        let timestamp_ns = self.start_time.elapsed().as_nanos() as u64;
        let idx = self.write_pos;

        self.frames[idx] = FaultFrame::new(cycle, timestamp_ns, phase_timings);
        self.frames[idx].set_inputs(inputs);
        self.frames[idx].set_outputs(outputs);
        self.frames[idx].set_fault(reason);
        self.fault_frame_index = Some(idx);

        self.write_pos = (self.write_pos + 1) % self.frames.len();
        self.frame_count = self.frame_count.saturating_add(1);
        self.frozen = true;
    }

    /// Freeze the recorder without recording a new fault frame.
    ///
    /// Use this when the fault was already recorded via `record_cycle`.
    pub fn freeze(&mut self) {
        self.frozen = true;
    }

    /// Get the fault frame if a fault was recorded.
    #[must_use]
    pub fn fault_frame(&self) -> Option<&FaultFrame> {
        self.fault_frame_index.map(|idx| &self.frames[idx])
    }

    /// Iterate over recorded frames in chronological order.
    ///
    /// Returns frames from oldest to newest, up to the fault frame.
    pub fn frames_chronological(&self) -> impl Iterator<Item = &FaultFrame> {
        let count = self.frame_count();
        let cap = self.frames.len();

        // Calculate the starting index for chronological iteration
        let start = if self.frame_count > cap {
            self.write_pos // Oldest frame is at write_pos (was just overwritten)
        } else {
            0
        };

        (0..count).map(move |i| {
            let idx = (start + i) % cap;
            &self.frames[idx]
        })
    }

    /// Get the N most recent frames before the fault.
    ///
    /// Returns frames in reverse chronological order (newest first).
    pub fn recent_frames(&self, count: usize) -> Vec<&FaultFrame> {
        let actual_count = count.min(self.frame_count());
        let cap = self.frames.len();

        (0..actual_count)
            .map(|i| {
                let idx = if self.write_pos == 0 {
                    cap - 1 - i
                } else {
                    (self.write_pos + cap - 1 - i) % cap
                };
                &self.frames[idx]
            })
            .filter(|f| f.valid)
            .collect()
    }

    /// Reset the recorder, clearing all frames and unfreezing.
    pub fn reset(&mut self) {
        for frame in self.frames.iter_mut() {
            *frame = FaultFrame::default();
        }
        self.write_pos = 0;
        self.frame_count = 0;
        self.start_time = std::time::Instant::now();
        self.frozen = false;
        self.fault_frame_index = None;
    }

    /// Get a summary of the recorded fault for logging.
    #[must_use]
    pub fn fault_summary(&self) -> Option<FaultSummary> {
        let fault_frame = self.fault_frame()?;

        Some(FaultSummary {
            cycle: fault_frame.cycle,
            reason: fault_frame.fault_reason,
            execution_time: fault_frame.phase_timings.total,
            io_read_time: fault_frame.phase_timings.io_read,
            logic_exec_time: fault_frame.phase_timings.logic_exec,
            io_write_time: fault_frame.phase_timings.io_write,
            wkc_mismatch: fault_frame
                .wkc
                .zip(fault_frame.expected_wkc)
                .map(|(actual, expected)| actual != expected)
                .unwrap_or(false),
            frames_available: self.frame_count(),
        })
    }
}

/// Summary of a recorded fault for reporting.
#[derive(Debug, Clone)]
pub struct FaultSummary {
    /// Cycle at which the fault occurred.
    pub cycle: u64,
    /// Reason for the fault.
    pub reason: FaultReason,
    /// Total execution time of the faulting cycle.
    pub execution_time: Duration,
    /// I/O read phase time.
    pub io_read_time: Duration,
    /// Logic execution phase time.
    pub logic_exec_time: Duration,
    /// I/O write phase time.
    pub io_write_time: Duration,
    /// Whether there was a WKC mismatch.
    pub wkc_mismatch: bool,
    /// Number of frames available for analysis.
    pub frames_available: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fault_recorder_creation() {
        let recorder = FaultRecorder::new(10);
        assert_eq!(recorder.capacity(), 10);
        assert_eq!(recorder.frame_count(), 0);
        assert!(!recorder.is_frozen());
    }

    #[test]
    fn test_record_cycle() {
        let mut recorder = FaultRecorder::new(10);
        let timings = CyclePhaseTimings {
            io_read: Duration::from_micros(10),
            logic_exec: Duration::from_micros(100),
            io_write: Duration::from_micros(10),
            fieldbus_exchange: Duration::ZERO,
            total: Duration::from_micros(120),
        };

        let frame = recorder.record_cycle(1, timings).unwrap();
        frame.set_inputs(&ProcessData::default());

        assert_eq!(recorder.frame_count(), 1);
        assert!(!recorder.is_frozen());
    }

    #[test]
    fn test_ring_buffer_wrapping() {
        let mut recorder = FaultRecorder::new(4);
        let timings = CyclePhaseTimings::default();

        for i in 0..10 {
            recorder.record_cycle(i, timings);
        }

        // Should have 4 frames (capacity), but 10 recorded
        assert_eq!(recorder.frame_count(), 4);

        // Most recent frames should be cycles 6-9
        let recent = recorder.recent_frames(4);
        assert_eq!(recent.len(), 4);
        assert_eq!(recent[0].cycle, 9); // Most recent
        assert_eq!(recent[3].cycle, 6); // Oldest in buffer
    }

    #[test]
    fn test_record_fault_and_freeze() {
        let mut recorder = FaultRecorder::new(10);
        let timings = CyclePhaseTimings::default();

        // Record some normal cycles
        for i in 0..5 {
            recorder.record_cycle(i, timings);
        }

        // Record fault - now creates a new frame with the fault cycle's data
        recorder.record_fault(5, FaultReason::CycleOverrun, timings);

        assert!(recorder.is_frozen());
        assert!(recorder.fault_frame().is_some());
        let fault_frame = recorder.fault_frame().unwrap();
        assert_eq!(fault_frame.fault_reason, FaultReason::CycleOverrun);
        assert_eq!(fault_frame.cycle, 5); // Verify correct cycle number

        // Further recording should fail
        assert!(recorder.record_cycle(6, timings).is_none());
    }

    #[test]
    fn test_fault_summary() {
        let mut recorder = FaultRecorder::new(10);
        let timings = CyclePhaseTimings {
            io_read: Duration::from_micros(10),
            logic_exec: Duration::from_micros(100),
            io_write: Duration::from_micros(10),
            fieldbus_exchange: Duration::ZERO,
            total: Duration::from_micros(1200), // Overrun
        };

        recorder.record_cycle(41, timings); // Previous cycle
        recorder.record_fault(42, FaultReason::CycleOverrun, timings);

        let summary = recorder.fault_summary().unwrap();
        assert_eq!(summary.cycle, 42);
        assert_eq!(summary.reason, FaultReason::CycleOverrun);
        assert_eq!(summary.logic_exec_time, Duration::from_micros(100));
    }

    #[test]
    fn test_chronological_iteration() {
        let mut recorder = FaultRecorder::new(4);
        let timings = CyclePhaseTimings::default();

        for i in 0..6 {
            recorder.record_cycle(i, timings);
        }

        let cycles: Vec<u64> = recorder.frames_chronological().map(|f| f.cycle).collect();
        // Should be [2, 3, 4, 5] after wrapping
        assert_eq!(cycles, vec![2, 3, 4, 5]);
    }

    #[test]
    fn test_reset() {
        let mut recorder = FaultRecorder::new(10);
        let timings = CyclePhaseTimings::default();

        for i in 0..5 {
            recorder.record_cycle(i, timings);
        }
        recorder.record_fault(5, FaultReason::WasmTrap, timings);

        assert!(recorder.is_frozen());

        recorder.reset();

        assert!(!recorder.is_frozen());
        assert_eq!(recorder.frame_count(), 0);
        assert!(recorder.fault_frame().is_none());
    }

    #[test]
    fn test_fault_frame_io_snapshot() {
        let mut frame = FaultFrame::default();

        let mut data = ProcessData::default();
        data.digital_inputs[0] = 0xDEADBEEF;
        data.analog_inputs[0] = 1234;
        data.analog_inputs[1] = -5678;

        frame.set_inputs(&data);

        // Verify digital inputs packed correctly
        assert_eq!(
            u32::from_le_bytes(frame.inputs[0..4].try_into().unwrap()),
            0xDEADBEEF
        );

        // Verify first analog input
        assert_eq!(
            i16::from_le_bytes(frame.inputs[4..6].try_into().unwrap()),
            1234
        );

        // Verify second analog input
        assert_eq!(
            i16::from_le_bytes(frame.inputs[6..8].try_into().unwrap()),
            -5678
        );
    }
}
