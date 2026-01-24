//! IEC 61131-3 bistable function blocks.
//!
//! Bistable elements are memory cells with set and reset inputs:
//! - [`Sr`] - Set-Reset flip-flop (set dominant)
//! - [`Rs`] - Reset-Set flip-flop (reset dominant)
//!
//! The difference between SR and RS is which input takes priority when both
//! SET1/S and RESET/R1 are TRUE simultaneously.

use serde::{Deserialize, Serialize};

/// Set-Reset flip-flop (SR) - Set dominant.
///
/// When both SET1 and RESET are TRUE, SET1 wins (output Q1 is TRUE).
///
/// # IEC 61131-3 Interface
///
/// ```text
/// FUNCTION_BLOCK SR
/// VAR_INPUT
///     SET1 : BOOL;   (* Set input - dominant *)
///     RESET : BOOL;  (* Reset input *)
/// END_VAR
/// VAR_OUTPUT
///     Q1 : BOOL;     (* Output *)
/// END_VAR
/// ```
///
/// # Logic
///
/// ```text
/// Q1 := SET1 OR (NOT RESET AND Q1)
/// ```
///
/// # Truth Table
///
/// | SET1 | RESET | Q1 (prev) | Q1 (new) |
/// |------|-------|-----------|----------|
/// |  0   |   0   |     0     |    0     |
/// |  0   |   0   |     1     |    1     |
/// |  0   |   1   |     X     |    0     |
/// |  1   |   0   |     X     |    1     |
/// |  1   |   1   |     X     |    1     | ← Set dominant
///
/// # Example
///
/// ```
/// use plc_stdlib::bistable::Sr;
///
/// let mut sr = Sr::new();
///
/// // Initially off
/// assert!(!sr.call(false, false));
///
/// // Set
/// assert!(sr.call(true, false));
///
/// // Stays set (memory)
/// assert!(sr.call(false, false));
///
/// // Reset
/// assert!(!sr.call(false, true));
///
/// // Set dominant - both inputs TRUE
/// assert!(sr.call(true, true));
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Sr {
    /// Output Q1.
    q1: bool,
}

impl Sr {
    /// Create a new SR instance.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new SR instance with initial state.
    #[must_use]
    pub fn with_state(initial: bool) -> Self {
        Self { q1: initial }
    }

    /// Execute one scan cycle.
    ///
    /// # Arguments
    ///
    /// * `set1` - Set input (dominant).
    /// * `reset` - Reset input.
    ///
    /// # Returns
    ///
    /// The output Q1.
    pub fn call(&mut self, set1: bool, reset: bool) -> bool {
        // Q1 := SET1 OR (NOT RESET AND Q1)
        self.q1 = set1 || (!reset && self.q1);
        self.q1
    }

    /// Get current output state.
    #[must_use]
    pub fn q1(&self) -> bool {
        self.q1
    }

    /// Reset to initial state.
    pub fn reset(&mut self) {
        self.q1 = false;
    }
}

/// Reset-Set flip-flop (RS) - Reset dominant.
///
/// When both S and R1 are TRUE, R1 wins (output Q1 is FALSE).
///
/// # IEC 61131-3 Interface
///
/// ```text
/// FUNCTION_BLOCK RS
/// VAR_INPUT
///     S : BOOL;    (* Set input *)
///     R1 : BOOL;   (* Reset input - dominant *)
/// END_VAR
/// VAR_OUTPUT
///     Q1 : BOOL;   (* Output *)
/// END_VAR
/// ```
///
/// # Logic
///
/// ```text
/// Q1 := NOT R1 AND (S OR Q1)
/// ```
///
/// # Truth Table
///
/// | S | R1 | Q1 (prev) | Q1 (new) |
/// |---|----|-----------| ---------|
/// | 0 |  0 |     0     |    0     |
/// | 0 |  0 |     1     |    1     |
/// | 0 |  1 |     X     |    0     |
/// | 1 |  0 |     X     |    1     |
/// | 1 |  1 |     X     |    0     | ← Reset dominant
///
/// # Example
///
/// ```
/// use plc_stdlib::bistable::Rs;
///
/// let mut rs = Rs::new();
///
/// // Initially off
/// assert!(!rs.call(false, false));
///
/// // Set
/// assert!(rs.call(true, false));
///
/// // Stays set (memory)
/// assert!(rs.call(false, false));
///
/// // Reset dominant - both inputs TRUE
/// assert!(!rs.call(true, true));
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Rs {
    /// Output Q1.
    q1: bool,
}

impl Rs {
    /// Create a new RS instance.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new RS instance with initial state.
    #[must_use]
    pub fn with_state(initial: bool) -> Self {
        Self { q1: initial }
    }

    /// Execute one scan cycle.
    ///
    /// # Arguments
    ///
    /// * `s` - Set input.
    /// * `r1` - Reset input (dominant).
    ///
    /// # Returns
    ///
    /// The output Q1.
    pub fn call(&mut self, s: bool, r1: bool) -> bool {
        // Q1 := NOT R1 AND (S OR Q1)
        self.q1 = !r1 && (s || self.q1);
        self.q1
    }

    /// Get current output state.
    #[must_use]
    pub fn q1(&self) -> bool {
        self.q1
    }

    /// Reset to initial state.
    pub fn reset(&mut self) {
        self.q1 = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sr_basic_operation() {
        let mut sr = Sr::new();

        // Initially off
        assert!(!sr.call(false, false));

        // Set
        assert!(sr.call(true, false));

        // Memory - stays set
        assert!(sr.call(false, false));
        assert!(sr.call(false, false));

        // Reset
        assert!(!sr.call(false, true));

        // Memory - stays reset
        assert!(!sr.call(false, false));
    }

    #[test]
    fn test_sr_set_dominant() {
        let mut sr = Sr::new();

        // Both inputs TRUE - SET wins
        assert!(sr.call(true, true));

        // Verify still set
        assert!(sr.call(false, false));
    }

    #[test]
    fn test_sr_with_initial_state() {
        let sr_off = Sr::with_state(false);
        let sr_on = Sr::with_state(true);

        assert!(!sr_off.q1());
        assert!(sr_on.q1());
    }

    #[test]
    fn test_rs_basic_operation() {
        let mut rs = Rs::new();

        // Initially off
        assert!(!rs.call(false, false));

        // Set
        assert!(rs.call(true, false));

        // Memory - stays set
        assert!(rs.call(false, false));
        assert!(rs.call(false, false));

        // Reset
        assert!(!rs.call(false, true));

        // Memory - stays reset
        assert!(!rs.call(false, false));
    }

    #[test]
    fn test_rs_reset_dominant() {
        let mut rs = Rs::new();

        // First set it
        rs.call(true, false);

        // Both inputs TRUE - RESET wins
        assert!(!rs.call(true, true));

        // Verify still reset
        assert!(!rs.call(false, false));
    }

    #[test]
    fn test_rs_with_initial_state() {
        let rs_off = Rs::with_state(false);
        let rs_on = Rs::with_state(true);

        assert!(!rs_off.q1());
        assert!(rs_on.q1());
    }

    #[test]
    fn test_sr_vs_rs_difference() {
        let mut sr = Sr::new();
        let mut rs = Rs::new();

        // Both inputs TRUE shows the difference
        let sr_result = sr.call(true, true);
        let rs_result = rs.call(true, true);

        assert!(sr_result, "SR should be TRUE (set dominant)");
        assert!(!rs_result, "RS should be FALSE (reset dominant)");
    }

    #[test]
    fn test_sr_reset_method() {
        let mut sr = Sr::with_state(true);
        assert!(sr.q1());

        sr.reset();
        assert!(!sr.q1());
    }

    #[test]
    fn test_rs_reset_method() {
        let mut rs = Rs::with_state(true);
        assert!(rs.q1());

        rs.reset();
        assert!(!rs.q1());
    }
}
