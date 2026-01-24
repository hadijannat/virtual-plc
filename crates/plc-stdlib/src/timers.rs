//! IEC 61131-3 standard timer function blocks.
//!
//! This module provides three timer types per the IEC 61131-3 standard:
//! - [`Ton`] - Timer On-Delay: Output goes TRUE after input has been TRUE for preset time
//! - [`Tof`] - Timer Off-Delay: Output stays TRUE for preset time after input goes FALSE
//! - [`Tp`] - Timer Pulse: Generates fixed-duration pulse on rising edge
//!
//! All timers use TIME (i64 nanoseconds) for time values.

use serde::{Deserialize, Serialize};

/// Timer On-Delay (TON).
///
/// Output Q goes TRUE when input IN has been TRUE for at least PT (preset time).
/// Output Q goes FALSE immediately when IN goes FALSE.
/// Elapsed time ET counts up while IN is TRUE, resets when IN is FALSE.
///
/// # IEC 61131-3 Interface
///
/// ```text
/// FUNCTION_BLOCK TON
/// VAR_INPUT
///     IN : BOOL;    (* Start input *)
///     PT : TIME;    (* Preset time *)
/// END_VAR
/// VAR_OUTPUT
///     Q : BOOL;     (* Output - TRUE when elapsed >= preset *)
///     ET : TIME;    (* Elapsed time *)
/// END_VAR
/// ```
///
/// # Timing Diagram
///
/// ```text
///       +------+     +---------------+
/// IN    |      |     |               |
///    ---+      +-----+               +----
///
///              +-----+          +----+
/// Q            |     |          |    |
///    ----------+     +----------+    +----
///              PT    PT
///
///       /------\     /--------------\
/// ET   /        \   /                \
///    -/          \_/                  \---
/// ```
///
/// # Example
///
/// ```
/// use plc_stdlib::timers::Ton;
///
/// let mut ton = Ton::new();
/// let pt = 1_000_000_000; // 1 second in nanoseconds
///
/// // Input FALSE - output FALSE
/// let (q, et) = ton.call(false, pt, 100_000_000);
/// assert!(!q);
/// assert_eq!(et, 0);
///
/// // Input TRUE - start timing
/// let (q, et) = ton.call(true, pt, 500_000_000);
/// assert!(!q); // Not yet reached preset
/// assert_eq!(et, 500_000_000);
///
/// // Continue timing
/// let (q, et) = ton.call(true, pt, 600_000_000);
/// assert!(q); // Now reached preset (500 + 600 = 1100ms > 1000ms)
/// assert_eq!(et, 1_000_000_000); // Capped at PT
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Ton {
    /// Output Q.
    q: bool,
    /// Elapsed time.
    et: i64,
    /// Previous IN state for edge detection.
    prev_in: bool,
}

impl Ton {
    /// Create a new TON timer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Execute one scan cycle.
    ///
    /// # Arguments
    ///
    /// * `input` - The IN input (start timing when TRUE).
    /// * `pt` - Preset time in nanoseconds.
    /// * `delta_t` - Time elapsed since last cycle in nanoseconds.
    ///
    /// # Returns
    ///
    /// A tuple of (Q output, ET elapsed time).
    pub fn call(&mut self, input: bool, pt: i64, delta_t: i64) -> (bool, i64) {
        if input {
            // Accumulate time while input is TRUE
            if self.et < pt {
                self.et = (self.et + delta_t).min(pt);
            }
            // Output TRUE when elapsed >= preset
            self.q = self.et >= pt;
        } else {
            // Reset when input goes FALSE
            self.q = false;
            self.et = 0;
        }

        self.prev_in = input;
        (self.q, self.et)
    }

    /// Get current Q output.
    #[must_use]
    pub fn q(&self) -> bool {
        self.q
    }

    /// Get current elapsed time.
    #[must_use]
    pub fn et(&self) -> i64 {
        self.et
    }

    /// Reset the timer.
    pub fn reset(&mut self) {
        self.q = false;
        self.et = 0;
        self.prev_in = false;
    }
}

/// Timer Off-Delay (TOF).
///
/// Output Q goes TRUE immediately when IN goes TRUE.
/// Output Q stays TRUE for PT (preset time) after IN goes FALSE.
/// Elapsed time ET counts up after IN goes FALSE.
///
/// # IEC 61131-3 Interface
///
/// ```text
/// FUNCTION_BLOCK TOF
/// VAR_INPUT
///     IN : BOOL;    (* Input *)
///     PT : TIME;    (* Preset time - delay before turning off *)
/// END_VAR
/// VAR_OUTPUT
///     Q : BOOL;     (* Output *)
///     ET : TIME;    (* Elapsed time since IN went FALSE *)
/// END_VAR
/// ```
///
/// # Timing Diagram
///
/// ```text
///       +------+     +---+
/// IN    |      |     |   |
///    ---+      +-----+   +----------------
///
///       +------------+   +-------+
/// Q     |            |   |       |
///    ---+            +---+       +---------
///              PT          PT
///
///              /-----\         /--\
/// ET          /       \       /    \
///    --------/         \-----/      \-----
/// ```
///
/// # Example
///
/// ```
/// use plc_stdlib::timers::Tof;
///
/// let mut tof = Tof::new();
/// let pt = 1_000_000_000; // 1 second
///
/// // Input TRUE - output immediately TRUE
/// let (q, et) = tof.call(true, pt, 100_000_000);
/// assert!(q);
/// assert_eq!(et, 0);
///
/// // Input FALSE - start off-delay timing
/// let (q, et) = tof.call(false, pt, 500_000_000);
/// assert!(q); // Still TRUE during delay
/// assert_eq!(et, 500_000_000);
///
/// // Continue timing past preset
/// let (q, et) = tof.call(false, pt, 600_000_000);
/// assert!(!q); // Now FALSE (500 + 600 > 1000)
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tof {
    /// Output Q.
    q: bool,
    /// Elapsed time.
    et: i64,
    /// Previous IN state for edge detection.
    prev_in: bool,
    /// Timer is running (started on falling edge of IN).
    running: bool,
}

impl Tof {
    /// Create a new TOF timer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Execute one scan cycle.
    ///
    /// # Arguments
    ///
    /// * `input` - The IN input.
    /// * `pt` - Preset time in nanoseconds.
    /// * `delta_t` - Time elapsed since last cycle in nanoseconds.
    ///
    /// # Returns
    ///
    /// A tuple of (Q output, ET elapsed time).
    pub fn call(&mut self, input: bool, pt: i64, delta_t: i64) -> (bool, i64) {
        if input {
            // Input TRUE - output TRUE, reset timer
            self.q = true;
            self.et = 0;
            self.running = false;
        } else if self.prev_in && !input {
            // Falling edge - start timer and accumulate first delta_t
            self.running = true;
            self.et = delta_t.min(pt);
            self.q = self.et < pt;
        } else if self.running {
            // Timer running
            self.et = (self.et + delta_t).min(pt);
            if self.et >= pt {
                self.q = false;
                self.running = false;
            }
        }

        self.prev_in = input;
        (self.q, self.et)
    }

    /// Get current Q output.
    #[must_use]
    pub fn q(&self) -> bool {
        self.q
    }

    /// Get current elapsed time.
    #[must_use]
    pub fn et(&self) -> i64 {
        self.et
    }

    /// Reset the timer.
    pub fn reset(&mut self) {
        self.q = false;
        self.et = 0;
        self.prev_in = false;
        self.running = false;
    }
}

/// Timer Pulse (TP).
///
/// Generates a pulse of duration PT on the rising edge of IN.
/// Once started, the pulse runs for exactly PT regardless of IN changes.
/// A new pulse can only start after the current pulse completes.
///
/// # IEC 61131-3 Interface
///
/// ```text
/// FUNCTION_BLOCK TP
/// VAR_INPUT
///     IN : BOOL;    (* Trigger input *)
///     PT : TIME;    (* Pulse duration *)
/// END_VAR
/// VAR_OUTPUT
///     Q : BOOL;     (* Pulse output *)
///     ET : TIME;    (* Elapsed time of pulse *)
/// END_VAR
/// ```
///
/// # Timing Diagram
///
/// ```text
///       +--+  +------+    +--+
/// IN    |  |  |      |    |  |
///    ---+  +--+      +----+  +------------
///
///       +-----+------+    +-----+
/// Q     |     |      |    |     |
///    ---+     +------+----+     +---------
///           PT              PT
///
///       /-----\            /-----\
/// ET   /       \          /       \
///    -/         \--------/         \------
/// ```
///
/// # Example
///
/// ```
/// use plc_stdlib::timers::Tp;
///
/// let mut tp = Tp::new();
/// let pt = 1_000_000_000; // 1 second pulse
///
/// // Rising edge - start pulse
/// let (q, et) = tp.call(true, pt, 100_000_000);
/// assert!(q);
///
/// // Input goes FALSE - pulse continues
/// let (q, et) = tp.call(false, pt, 400_000_000);
/// assert!(q);
///
/// // Pulse completes
/// let (q, et) = tp.call(false, pt, 600_000_000);
/// assert!(!q); // Pulse ended
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tp {
    /// Output Q.
    q: bool,
    /// Elapsed time.
    et: i64,
    /// Previous IN state for edge detection.
    prev_in: bool,
    /// Pulse is running.
    running: bool,
}

impl Tp {
    /// Create a new TP timer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Execute one scan cycle.
    ///
    /// # Arguments
    ///
    /// * `input` - The IN trigger input.
    /// * `pt` - Pulse duration in nanoseconds.
    /// * `delta_t` - Time elapsed since last cycle in nanoseconds.
    ///
    /// # Returns
    ///
    /// A tuple of (Q output, ET elapsed time).
    pub fn call(&mut self, input: bool, pt: i64, delta_t: i64) -> (bool, i64) {
        // Detect rising edge to start pulse (only if not already running)
        if input && !self.prev_in && !self.running {
            self.running = true;
            self.q = true;
            self.et = 0;
        }

        // If pulse is running, accumulate time
        if self.running {
            self.et = (self.et + delta_t).min(pt);
            if self.et >= pt {
                self.q = false;
                self.running = false;
                self.et = 0;
            }
        }

        self.prev_in = input;
        (self.q, self.et)
    }

    /// Get current Q output.
    #[must_use]
    pub fn q(&self) -> bool {
        self.q
    }

    /// Get current elapsed time.
    #[must_use]
    pub fn et(&self) -> i64 {
        self.et
    }

    /// Check if pulse is running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Reset the timer.
    pub fn reset(&mut self) {
        self.q = false;
        self.et = 0;
        self.prev_in = false;
        self.running = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Constants for time values (nanoseconds)
    const MS: i64 = 1_000_000;
    const SEC: i64 = 1_000_000_000;

    // ==================== TON Tests ====================

    #[test]
    fn test_ton_basic_operation() {
        let mut ton = Ton::new();
        let pt = SEC; // 1 second preset

        // Input FALSE - no timing
        let (q, et) = ton.call(false, pt, 100 * MS);
        assert!(!q);
        assert_eq!(et, 0);

        // Input TRUE - start timing
        let (q, et) = ton.call(true, pt, 400 * MS);
        assert!(!q);
        assert_eq!(et, 400 * MS);

        // Continue timing
        let (q, et) = ton.call(true, pt, 400 * MS);
        assert!(!q);
        assert_eq!(et, 800 * MS);

        // Reach preset
        let (q, et) = ton.call(true, pt, 300 * MS);
        assert!(q);
        assert_eq!(et, SEC); // Capped at PT

        // Input FALSE - immediate reset
        let (q, et) = ton.call(false, pt, 100 * MS);
        assert!(!q);
        assert_eq!(et, 0);
    }

    #[test]
    fn test_ton_retrigger() {
        let mut ton = Ton::new();
        let pt = SEC;

        // Start timing
        ton.call(true, pt, 500 * MS);

        // Input goes FALSE before preset
        let (q, et) = ton.call(false, pt, 100 * MS);
        assert!(!q);
        assert_eq!(et, 0);

        // Start timing again
        let (q, et) = ton.call(true, pt, 600 * MS);
        assert!(!q);
        assert_eq!(et, 600 * MS);
    }

    #[test]
    fn test_ton_stays_on() {
        let mut ton = Ton::new();
        let pt = SEC;

        // Reach preset
        ton.call(true, pt, SEC);
        ton.call(true, pt, SEC);

        // Stays on while input is TRUE
        let (q, _) = ton.call(true, pt, 100 * MS);
        assert!(q);

        let (q, _) = ton.call(true, pt, 100 * MS);
        assert!(q);
    }

    // ==================== TOF Tests ====================

    #[test]
    fn test_tof_basic_operation() {
        let mut tof = Tof::new();
        let pt = SEC;

        // Input TRUE - output immediately TRUE
        let (q, et) = tof.call(true, pt, 100 * MS);
        assert!(q);
        assert_eq!(et, 0);

        // Input FALSE - start off-delay
        let (q, et) = tof.call(false, pt, 400 * MS);
        assert!(q); // Still TRUE during delay
        assert_eq!(et, 400 * MS);

        // Continue delay
        let (q, et) = tof.call(false, pt, 400 * MS);
        assert!(q);
        assert_eq!(et, 800 * MS);

        // Delay expires
        let (q, et) = tof.call(false, pt, 300 * MS);
        assert!(!q);
        assert_eq!(et, SEC);
    }

    #[test]
    fn test_tof_retrigger_during_delay() {
        let mut tof = Tof::new();
        let pt = SEC;

        // Input TRUE then FALSE
        tof.call(true, pt, 100 * MS);
        tof.call(false, pt, 500 * MS);

        // Input TRUE again during delay - resets
        let (q, et) = tof.call(true, pt, 100 * MS);
        assert!(q);
        assert_eq!(et, 0);
    }

    #[test]
    fn test_tof_starts_off() {
        let mut tof = Tof::new();
        let pt = SEC;

        // Initially FALSE - output FALSE
        let (q, et) = tof.call(false, pt, 100 * MS);
        assert!(!q);
        assert_eq!(et, 0);
    }

    // ==================== TP Tests ====================

    #[test]
    fn test_tp_basic_pulse() {
        let mut tp = Tp::new();
        let pt = SEC;

        // No pulse initially
        let (q, et) = tp.call(false, pt, 100 * MS);
        assert!(!q);
        assert_eq!(et, 0);

        // Rising edge - start pulse
        let (q, et) = tp.call(true, pt, 100 * MS);
        assert!(q);
        assert_eq!(et, 100 * MS);

        // Pulse continues
        let (q, et) = tp.call(true, pt, 400 * MS);
        assert!(q);
        assert_eq!(et, 500 * MS);

        // Input goes FALSE - pulse continues
        let (q, et) = tp.call(false, pt, 400 * MS);
        assert!(q);
        assert_eq!(et, 900 * MS);

        // Pulse ends
        let (q, et) = tp.call(false, pt, 200 * MS);
        assert!(!q);
        assert_eq!(et, 0); // Reset after pulse
    }

    #[test]
    fn test_tp_ignores_input_during_pulse() {
        let mut tp = Tp::new();
        let pt = SEC;

        // Start pulse
        tp.call(true, pt, 100 * MS);

        // Input toggles during pulse - ignored
        tp.call(false, pt, 200 * MS);
        tp.call(true, pt, 200 * MS);
        tp.call(false, pt, 200 * MS);

        // Pulse still running
        let (q, et) = tp.call(false, pt, 200 * MS);
        assert!(q);
        assert_eq!(et, 900 * MS);
    }

    #[test]
    fn test_tp_new_pulse_after_complete() {
        let mut tp = Tp::new();
        let pt = 500 * MS;

        // First pulse
        tp.call(true, pt, 100 * MS);
        tp.call(false, pt, 500 * MS); // Pulse ends

        // New rising edge - new pulse
        let (q, _) = tp.call(true, pt, 100 * MS);
        assert!(q);
    }

    #[test]
    fn test_tp_no_retrigger_during_pulse() {
        let mut tp = Tp::new();
        let pt = SEC;

        // Start pulse
        tp.call(true, pt, 100 * MS);

        // Try to retrigger during pulse
        tp.call(false, pt, 100 * MS);
        let start_et = tp.et();

        tp.call(true, pt, 100 * MS); // Rising edge during pulse

        // Time should continue, not restart
        assert!(tp.et() > start_et);
    }

    // ==================== Reset Tests ====================

    #[test]
    fn test_ton_reset() {
        let mut ton = Ton::new();
        ton.call(true, SEC, 500 * MS);

        ton.reset();

        assert!(!ton.q());
        assert_eq!(ton.et(), 0);
    }

    #[test]
    fn test_tof_reset() {
        let mut tof = Tof::new();
        tof.call(true, SEC, 100 * MS);
        tof.call(false, SEC, 500 * MS);

        tof.reset();

        assert!(!tof.q());
        assert_eq!(tof.et(), 0);
    }

    #[test]
    fn test_tp_reset() {
        let mut tp = Tp::new();
        tp.call(true, SEC, 500 * MS);

        tp.reset();

        assert!(!tp.q());
        assert_eq!(tp.et(), 0);
        assert!(!tp.is_running());
    }
}
