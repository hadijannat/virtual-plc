//! IEC 61131-3 edge detection function blocks.
//!
//! Edge triggers detect transitions in boolean signals:
//! - [`RTrig`] - Rising edge (FALSE→TRUE transition)
//! - [`FTrig`] - Falling edge (TRUE→FALSE transition)
//!
//! These are fundamental building blocks used by timers, counters, and
//! application logic that needs to respond to signal changes rather than levels.

use serde::{Deserialize, Serialize};

/// Rising edge trigger (R_TRIG).
///
/// Detects a rising edge (FALSE to TRUE transition) on the CLK input.
/// Output Q is TRUE for exactly one scan cycle when the rising edge occurs.
///
/// # IEC 61131-3 Interface
///
/// ```text
/// FUNCTION_BLOCK R_TRIG
/// VAR_INPUT
///     CLK : BOOL;  (* Signal to monitor *)
/// END_VAR
/// VAR_OUTPUT
///     Q : BOOL;    (* TRUE for one cycle on rising edge *)
/// END_VAR
/// ```
///
/// # Example
///
/// ```
/// use plc_stdlib::triggers::RTrig;
///
/// let mut rtrig = RTrig::new();
///
/// // Initial state - no edge
/// assert!(!rtrig.call(false));
///
/// // Rising edge detected
/// assert!(rtrig.call(true));
///
/// // Stays high - no edge
/// assert!(!rtrig.call(true));
///
/// // Falls - no rising edge
/// assert!(!rtrig.call(false));
///
/// // Rising edge detected again
/// assert!(rtrig.call(true));
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RTrig {
    /// Previous CLK value (M in IEC standard).
    prev_clk: bool,
}

impl RTrig {
    /// Create a new R_TRIG instance.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Execute one scan cycle.
    ///
    /// # Arguments
    ///
    /// * `clk` - The signal to monitor for rising edges.
    ///
    /// # Returns
    ///
    /// `true` if a rising edge was detected (CLK transitioned from FALSE to TRUE).
    pub fn call(&mut self, clk: bool) -> bool {
        // Q := CLK AND NOT M;
        // M := CLK;
        let q = clk && !self.prev_clk;
        self.prev_clk = clk;
        q
    }

    /// Reset the trigger to initial state.
    pub fn reset(&mut self) {
        self.prev_clk = false;
    }

    /// Get the previous CLK value (for diagnostics).
    #[must_use]
    pub fn prev_clk(&self) -> bool {
        self.prev_clk
    }
}

/// Falling edge trigger (F_TRIG).
///
/// Detects a falling edge (TRUE to FALSE transition) on the CLK input.
/// Output Q is TRUE for exactly one scan cycle when the falling edge occurs.
///
/// # IEC 61131-3 Interface
///
/// ```text
/// FUNCTION_BLOCK F_TRIG
/// VAR_INPUT
///     CLK : BOOL;  (* Signal to monitor *)
/// END_VAR
/// VAR_OUTPUT
///     Q : BOOL;    (* TRUE for one cycle on falling edge *)
/// END_VAR
/// ```
///
/// # Example
///
/// ```
/// use plc_stdlib::triggers::FTrig;
///
/// let mut ftrig = FTrig::new();
///
/// // Initial state - no edge
/// assert!(!ftrig.call(false));
///
/// // Rising - no falling edge
/// assert!(!ftrig.call(true));
///
/// // Falling edge detected
/// assert!(ftrig.call(false));
///
/// // Stays low - no edge
/// assert!(!ftrig.call(false));
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FTrig {
    /// Previous CLK value (M in IEC standard).
    prev_clk: bool,
}

impl FTrig {
    /// Create a new F_TRIG instance.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Execute one scan cycle.
    ///
    /// # Arguments
    ///
    /// * `clk` - The signal to monitor for falling edges.
    ///
    /// # Returns
    ///
    /// `true` if a falling edge was detected (CLK transitioned from TRUE to FALSE).
    pub fn call(&mut self, clk: bool) -> bool {
        // Q := NOT CLK AND M;
        // M := CLK;
        let q = !clk && self.prev_clk;
        self.prev_clk = clk;
        q
    }

    /// Reset the trigger to initial state.
    pub fn reset(&mut self) {
        self.prev_clk = false;
    }

    /// Get the previous CLK value (for diagnostics).
    #[must_use]
    pub fn prev_clk(&self) -> bool {
        self.prev_clk
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rtrig_detects_rising_edge() {
        let mut rtrig = RTrig::new();

        // No edge initially
        assert!(!rtrig.call(false));

        // Rising edge
        assert!(rtrig.call(true));

        // No edge while high
        assert!(!rtrig.call(true));
        assert!(!rtrig.call(true));

        // Falling - no rising edge
        assert!(!rtrig.call(false));

        // Rising edge again
        assert!(rtrig.call(true));
    }

    #[test]
    fn test_rtrig_pulse_train() {
        let mut rtrig = RTrig::new();
        let inputs = [false, true, false, true, false, true, true, false];
        let expected = [false, true, false, true, false, true, false, false];

        for (i, (&input, &exp)) in inputs.iter().zip(expected.iter()).enumerate() {
            assert_eq!(rtrig.call(input), exp, "Mismatch at cycle {}", i);
        }
    }

    #[test]
    fn test_ftrig_detects_falling_edge() {
        let mut ftrig = FTrig::new();

        // No edge initially
        assert!(!ftrig.call(false));

        // Rising - no falling edge
        assert!(!ftrig.call(true));

        // Falling edge
        assert!(ftrig.call(false));

        // No edge while low
        assert!(!ftrig.call(false));

        // Rising then falling
        assert!(!ftrig.call(true));
        assert!(ftrig.call(false));
    }

    #[test]
    fn test_ftrig_pulse_train() {
        let mut ftrig = FTrig::new();
        let inputs = [false, true, false, true, false, false, true, false];
        let expected = [false, false, true, false, true, false, false, true];

        for (i, (&input, &exp)) in inputs.iter().zip(expected.iter()).enumerate() {
            assert_eq!(ftrig.call(input), exp, "Mismatch at cycle {}", i);
        }
    }

    #[test]
    fn test_rtrig_reset() {
        let mut rtrig = RTrig::new();

        rtrig.call(true);
        assert!(rtrig.prev_clk());

        rtrig.reset();
        assert!(!rtrig.prev_clk());

        // After reset, rising edge should be detected again
        assert!(rtrig.call(true));
    }

    #[test]
    fn test_ftrig_reset() {
        let mut ftrig = FTrig::new();

        ftrig.call(true);
        assert!(ftrig.prev_clk());

        ftrig.reset();
        assert!(!ftrig.prev_clk());
    }
}
