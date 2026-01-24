//! IEC 61131-3 standard counter function blocks.
//!
//! This module provides three counter types per the IEC 61131-3 standard:
//! - [`Ctu`] - Counter Up: Counts up on rising edge of CU input
//! - [`Ctd`] - Counter Down: Counts down on rising edge of CD input
//! - [`Ctud`] - Counter Up/Down: Combined bidirectional counter
//!
//! Counters use edge detection internally to count on signal transitions.

use serde::{Deserialize, Serialize};

/// Counter Up (CTU).
///
/// Counts up on each rising edge of CU input.
/// Output Q is TRUE when CV (current value) >= PV (preset value).
/// Reset R sets CV to 0.
///
/// # IEC 61131-3 Interface
///
/// ```text
/// FUNCTION_BLOCK CTU
/// VAR_INPUT
///     CU : BOOL;    (* Count up input - counts on rising edge *)
///     R : BOOL;     (* Reset input *)
///     PV : INT;     (* Preset value *)
/// END_VAR
/// VAR_OUTPUT
///     Q : BOOL;     (* Output - TRUE when CV >= PV *)
///     CV : INT;     (* Current value *)
/// END_VAR
/// ```
///
/// # Example
///
/// ```
/// use plc_stdlib::counters::Ctu;
///
/// let mut ctu = Ctu::new();
/// let pv = 3;
///
/// // Initial state
/// let (q, cv) = ctu.call(false, false, pv);
/// assert!(!q);
/// assert_eq!(cv, 0);
///
/// // Count up on rising edges
/// let (q, cv) = ctu.call(true, false, pv);  // Rising edge
/// assert!(!q);
/// assert_eq!(cv, 1);
///
/// let (q, cv) = ctu.call(false, false, pv); // Falling - no count
/// let (q, cv) = ctu.call(true, false, pv);  // Rising edge
/// assert!(!q);
/// assert_eq!(cv, 2);
///
/// let (q, cv) = ctu.call(false, false, pv);
/// let (q, cv) = ctu.call(true, false, pv);  // Rising edge - reaches preset
/// assert!(q); // CV >= PV
/// assert_eq!(cv, 3);
///
/// // Reset
/// let (q, cv) = ctu.call(false, true, pv);
/// assert!(!q);
/// assert_eq!(cv, 0);
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Ctu {
    /// Current value.
    cv: i32,
    /// Output Q.
    q: bool,
    /// Previous CU state for edge detection.
    prev_cu: bool,
}

impl Ctu {
    /// Create a new CTU counter.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Execute one scan cycle.
    ///
    /// # Arguments
    ///
    /// * `cu` - Count up input (counts on rising edge).
    /// * `r` - Reset input.
    /// * `pv` - Preset value.
    ///
    /// # Returns
    ///
    /// A tuple of (Q output, CV current value).
    pub fn call(&mut self, cu: bool, r: bool, pv: i32) -> (bool, i32) {
        if r {
            // Reset takes priority
            self.cv = 0;
        } else if cu && !self.prev_cu && self.cv < i32::MAX {
            // Rising edge on CU - count up
            self.cv += 1;
        }

        self.q = self.cv >= pv;
        self.prev_cu = cu;
        (self.q, self.cv)
    }

    /// Get current value.
    #[must_use]
    pub fn cv(&self) -> i32 {
        self.cv
    }

    /// Get Q output.
    #[must_use]
    pub fn q(&self) -> bool {
        self.q
    }

    /// Reset the counter.
    pub fn reset(&mut self) {
        self.cv = 0;
        self.q = false;
        self.prev_cu = false;
    }
}

/// Counter Down (CTD).
///
/// Counts down on each rising edge of CD input.
/// Output Q is TRUE when CV (current value) <= 0.
/// Load LD sets CV to PV (preset value).
///
/// # IEC 61131-3 Interface
///
/// ```text
/// FUNCTION_BLOCK CTD
/// VAR_INPUT
///     CD : BOOL;    (* Count down input - counts on rising edge *)
///     LD : BOOL;    (* Load input - loads PV into CV *)
///     PV : INT;     (* Preset value *)
/// END_VAR
/// VAR_OUTPUT
///     Q : BOOL;     (* Output - TRUE when CV <= 0 *)
///     CV : INT;     (* Current value *)
/// END_VAR
/// ```
///
/// # Example
///
/// ```
/// use plc_stdlib::counters::Ctd;
///
/// let mut ctd = Ctd::new();
/// let pv = 3;
///
/// // Load preset value
/// let (q, cv) = ctd.call(false, true, pv);
/// assert!(!q);
/// assert_eq!(cv, 3);
///
/// // Count down on rising edges
/// let (q, cv) = ctd.call(true, false, pv);  // Rising edge
/// assert!(!q);
/// assert_eq!(cv, 2);
///
/// let (q, cv) = ctd.call(false, false, pv);
/// let (q, cv) = ctd.call(true, false, pv);  // Rising edge
/// assert!(!q);
/// assert_eq!(cv, 1);
///
/// let (q, cv) = ctd.call(false, false, pv);
/// let (q, cv) = ctd.call(true, false, pv);  // Rising edge - reaches zero
/// assert!(q); // CV <= 0
/// assert_eq!(cv, 0);
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Ctd {
    /// Current value.
    cv: i32,
    /// Output Q.
    q: bool,
    /// Previous CD state for edge detection.
    prev_cd: bool,
}

impl Ctd {
    /// Create a new CTD counter.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Execute one scan cycle.
    ///
    /// # Arguments
    ///
    /// * `cd` - Count down input (counts on rising edge).
    /// * `ld` - Load input (loads PV into CV).
    /// * `pv` - Preset value.
    ///
    /// # Returns
    ///
    /// A tuple of (Q output, CV current value).
    pub fn call(&mut self, cd: bool, ld: bool, pv: i32) -> (bool, i32) {
        if ld {
            // Load takes priority
            self.cv = pv;
        } else if cd && !self.prev_cd && self.cv > i32::MIN {
            // Rising edge on CD - count down
            self.cv -= 1;
        }

        self.q = self.cv <= 0;
        self.prev_cd = cd;
        (self.q, self.cv)
    }

    /// Get current value.
    #[must_use]
    pub fn cv(&self) -> i32 {
        self.cv
    }

    /// Get Q output.
    #[must_use]
    pub fn q(&self) -> bool {
        self.q
    }

    /// Reset the counter.
    pub fn reset(&mut self) {
        self.cv = 0;
        self.q = true; // Q is TRUE when CV <= 0
        self.prev_cd = false;
    }
}

/// Counter Up/Down (CTUD).
///
/// Bidirectional counter: counts up on CU rising edge, down on CD rising edge.
/// QU is TRUE when CV >= PV.
/// QD is TRUE when CV <= 0.
/// R resets CV to 0, LD loads PV into CV.
///
/// # IEC 61131-3 Interface
///
/// ```text
/// FUNCTION_BLOCK CTUD
/// VAR_INPUT
///     CU : BOOL;    (* Count up input *)
///     CD : BOOL;    (* Count down input *)
///     R : BOOL;     (* Reset input *)
///     LD : BOOL;    (* Load input *)
///     PV : INT;     (* Preset value *)
/// END_VAR
/// VAR_OUTPUT
///     QU : BOOL;    (* Output up - TRUE when CV >= PV *)
///     QD : BOOL;    (* Output down - TRUE when CV <= 0 *)
///     CV : INT;     (* Current value *)
/// END_VAR
/// ```
///
/// # Example
///
/// ```
/// use plc_stdlib::counters::Ctud;
///
/// let mut ctud = Ctud::new();
/// let pv = 5;
///
/// // Initial state
/// let (qu, qd, cv) = ctud.call(false, false, false, false, pv);
/// assert!(!qu);
/// assert!(qd); // CV <= 0
/// assert_eq!(cv, 0);
///
/// // Count up
/// let (qu, qd, cv) = ctud.call(true, false, false, false, pv);
/// assert!(!qu);
/// assert!(!qd);
/// assert_eq!(cv, 1);
///
/// // Load preset
/// let (qu, qd, cv) = ctud.call(false, false, false, true, pv);
/// assert!(qu); // CV >= PV
/// assert!(!qd);
/// assert_eq!(cv, 5);
///
/// // Reset
/// let (qu, qd, cv) = ctud.call(false, false, true, false, pv);
/// assert!(!qu);
/// assert!(qd);
/// assert_eq!(cv, 0);
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Ctud {
    /// Current value.
    cv: i32,
    /// Output up.
    qu: bool,
    /// Output down.
    qd: bool,
    /// Previous CU state for edge detection.
    prev_cu: bool,
    /// Previous CD state for edge detection.
    prev_cd: bool,
}

impl Ctud {
    /// Create a new CTUD counter.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Execute one scan cycle.
    ///
    /// # Arguments
    ///
    /// * `cu` - Count up input (counts on rising edge).
    /// * `cd` - Count down input (counts on rising edge).
    /// * `r` - Reset input (sets CV to 0).
    /// * `ld` - Load input (sets CV to PV).
    /// * `pv` - Preset value.
    ///
    /// # Returns
    ///
    /// A tuple of (QU up output, QD down output, CV current value).
    #[allow(clippy::fn_params_excessive_bools)]
    pub fn call(&mut self, cu: bool, cd: bool, r: bool, ld: bool, pv: i32) -> (bool, bool, i32) {
        if r {
            // Reset takes priority
            self.cv = 0;
        } else if ld {
            // Load
            self.cv = pv;
        } else {
            // Count up on CU rising edge
            if cu && !self.prev_cu && self.cv < i32::MAX {
                self.cv += 1;
            }
            // Count down on CD rising edge
            if cd && !self.prev_cd && self.cv > i32::MIN {
                self.cv -= 1;
            }
        }

        self.qu = self.cv >= pv;
        self.qd = self.cv <= 0;
        self.prev_cu = cu;
        self.prev_cd = cd;
        (self.qu, self.qd, self.cv)
    }

    /// Get current value.
    #[must_use]
    pub fn cv(&self) -> i32 {
        self.cv
    }

    /// Get QU (up) output.
    #[must_use]
    pub fn qu(&self) -> bool {
        self.qu
    }

    /// Get QD (down) output.
    #[must_use]
    pub fn qd(&self) -> bool {
        self.qd
    }

    /// Reset the counter.
    pub fn reset(&mut self) {
        self.cv = 0;
        self.qu = false;
        self.qd = true;
        self.prev_cu = false;
        self.prev_cd = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== CTU Tests ====================

    #[test]
    fn test_ctu_counts_on_rising_edge() {
        let mut ctu = Ctu::new();
        let pv = 5;

        // Initial state
        assert_eq!(ctu.call(false, false, pv), (false, 0));

        // Rising edge - count
        assert_eq!(ctu.call(true, false, pv), (false, 1));

        // Stays high - no count
        assert_eq!(ctu.call(true, false, pv), (false, 1));

        // Falling - no count
        assert_eq!(ctu.call(false, false, pv), (false, 1));

        // Rising edge - count again
        assert_eq!(ctu.call(true, false, pv), (false, 2));
    }

    #[test]
    fn test_ctu_reaches_preset() {
        let mut ctu = Ctu::new();
        let pv = 3;

        // Count to preset
        ctu.call(true, false, pv);
        ctu.call(false, false, pv);
        ctu.call(true, false, pv);
        ctu.call(false, false, pv);
        let (q, cv) = ctu.call(true, false, pv);

        assert!(q);
        assert_eq!(cv, 3);
    }

    #[test]
    fn test_ctu_continues_past_preset() {
        let mut ctu = Ctu::new();
        let pv = 2;

        // Count past preset
        ctu.call(true, false, pv);
        ctu.call(false, false, pv);
        ctu.call(true, false, pv);
        ctu.call(false, false, pv);
        let (q, cv) = ctu.call(true, false, pv);

        assert!(q);
        assert_eq!(cv, 3); // Continues past preset
    }

    #[test]
    fn test_ctu_reset() {
        let mut ctu = Ctu::new();
        let pv = 5;

        // Count up
        ctu.call(true, false, pv);
        ctu.call(false, false, pv);
        ctu.call(true, false, pv);

        // Reset
        let (q, cv) = ctu.call(false, true, pv);
        assert!(!q);
        assert_eq!(cv, 0);
    }

    #[test]
    fn test_ctu_reset_priority() {
        let mut ctu = Ctu::new();
        let pv = 5;

        // Count up
        ctu.call(true, false, pv);
        ctu.call(false, false, pv);

        // Reset with CU also high - reset wins
        let (_, cv) = ctu.call(true, true, pv);
        assert_eq!(cv, 0);
    }

    // ==================== CTD Tests ====================

    #[test]
    fn test_ctd_counts_down() {
        let mut ctd = Ctd::new();
        let pv = 5;

        // Load preset
        ctd.call(false, true, pv);
        assert_eq!(ctd.cv(), 5);

        // Count down
        ctd.call(true, false, pv);
        assert_eq!(ctd.cv(), 4);

        ctd.call(false, false, pv);
        ctd.call(true, false, pv);
        assert_eq!(ctd.cv(), 3);
    }

    #[test]
    fn test_ctd_reaches_zero() {
        let mut ctd = Ctd::new();
        let pv = 2;

        // Load and count down
        ctd.call(false, true, pv);
        ctd.call(true, false, pv);
        ctd.call(false, false, pv);
        let (q, cv) = ctd.call(true, false, pv);

        assert!(q); // CV <= 0
        assert_eq!(cv, 0);
    }

    #[test]
    fn test_ctd_continues_negative() {
        let mut ctd = Ctd::new();
        let pv = 1;

        // Load, count down past zero
        ctd.call(false, true, pv);
        ctd.call(true, false, pv);
        ctd.call(false, false, pv);
        let (q, cv) = ctd.call(true, false, pv);

        assert!(q);
        assert_eq!(cv, -1);
    }

    // ==================== CTUD Tests ====================

    #[test]
    fn test_ctud_counts_up() {
        let mut ctud = Ctud::new();
        let pv = 5;

        // Count up
        let (_, _, cv) = ctud.call(true, false, false, false, pv);
        assert_eq!(cv, 1);

        ctud.call(false, false, false, false, pv);
        let (_, _, cv) = ctud.call(true, false, false, false, pv);
        assert_eq!(cv, 2);
    }

    #[test]
    fn test_ctud_counts_down() {
        let mut ctud = Ctud::new();
        let pv = 5;

        // Load then count down
        ctud.call(false, false, false, true, pv);
        let (_, _, cv) = ctud.call(false, true, false, false, pv);
        assert_eq!(cv, 4);
    }

    #[test]
    fn test_ctud_bidirectional() {
        let mut ctud = Ctud::new();
        let pv = 5;

        // Count up twice
        ctud.call(true, false, false, false, pv);
        ctud.call(false, false, false, false, pv);
        ctud.call(true, false, false, false, pv);
        assert_eq!(ctud.cv(), 2);

        // Count down once
        ctud.call(false, true, false, false, pv);
        assert_eq!(ctud.cv(), 1);

        // Count up once
        ctud.call(true, false, false, false, pv);
        assert_eq!(ctud.cv(), 2);
    }

    #[test]
    fn test_ctud_outputs() {
        let mut ctud = Ctud::new();
        let pv = 3;

        // Initial - QD is true (CV <= 0)
        let (qu, qd, _) = ctud.call(false, false, false, false, pv);
        assert!(!qu);
        assert!(qd);

        // Count up to preset
        ctud.call(true, false, false, false, pv);
        ctud.call(false, false, false, false, pv);
        ctud.call(true, false, false, false, pv);
        ctud.call(false, false, false, false, pv);
        let (qu, qd, cv) = ctud.call(true, false, false, false, pv);

        assert!(qu); // CV >= PV
        assert!(!qd);
        assert_eq!(cv, 3);
    }

    #[test]
    fn test_ctud_reset() {
        let mut ctud = Ctud::new();
        let pv = 5;

        // Count up
        ctud.call(true, false, false, false, pv);
        ctud.call(false, false, false, false, pv);
        ctud.call(true, false, false, false, pv);

        // Reset
        let (qu, qd, cv) = ctud.call(false, false, true, false, pv);
        assert!(!qu);
        assert!(qd);
        assert_eq!(cv, 0);
    }

    #[test]
    fn test_ctud_load() {
        let mut ctud = Ctud::new();
        let pv = 5;

        // Load preset
        let (qu, qd, cv) = ctud.call(false, false, false, true, pv);
        assert!(qu); // CV >= PV
        assert!(!qd);
        assert_eq!(cv, 5);
    }

    #[test]
    fn test_ctud_reset_priority() {
        let mut ctud = Ctud::new();
        let pv = 5;

        // Load first
        ctud.call(false, false, false, true, pv);

        // Reset with LD also high - reset wins
        let (_, _, cv) = ctud.call(false, false, true, true, pv);
        assert_eq!(cv, 0);
    }

    #[test]
    fn test_ctud_simultaneous_count() {
        let mut ctud = Ctud::new();
        let pv = 5;

        // Start at 2
        ctud.call(true, false, false, false, pv);
        ctud.call(false, false, false, false, pv);
        ctud.call(true, false, false, false, pv);
        assert_eq!(ctud.cv(), 2);

        // Simultaneous CU and CD rising edge
        ctud.call(false, false, false, false, pv);
        let (_, _, cv) = ctud.call(true, true, false, false, pv);

        // Both count - net zero change
        assert_eq!(cv, 2);
    }
}
