//! Runtime state machine for PLC lifecycle management.
//!
//! State transitions follow IEC-inspired lifecycle:
//! BOOT → INIT → PRE_OP → RUN → FAULT → SAFE_STOP
//!
//! Fault transitions are allowed from most states to ensure
//! rapid response to error conditions.

use crate::error::{PlcError, PlcResult};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Runtime states for the PLC lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RuntimeState {
    /// Initial power-on state; hardware discovery.
    #[default]
    Boot,
    /// Configuration loading and validation.
    Init,
    /// Pre-operational: fieldbus slaves configured, logic loaded.
    PreOp,
    /// Normal cyclic operation.
    Run,
    /// Fault detected; outputs may be in undefined state.
    Fault,
    /// Safe shutdown: outputs set to safe values.
    SafeStop,
}

impl fmt::Display for RuntimeState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Boot => write!(f, "BOOT"),
            Self::Init => write!(f, "INIT"),
            Self::PreOp => write!(f, "PRE_OP"),
            Self::Run => write!(f, "RUN"),
            Self::Fault => write!(f, "FAULT"),
            Self::SafeStop => write!(f, "SAFE_STOP"),
        }
    }
}

impl RuntimeState {
    /// Check if a transition to `target` is valid from the current state.
    #[must_use]
    pub fn can_transition_to(&self, target: RuntimeState) -> bool {
        use RuntimeState::{Boot, Fault, Init, PreOp, Run, SafeStop};

        matches!(
            (self, target),
            // Normal forward progression
            (Boot, Init)
                | (Init, PreOp)
                | (PreOp, Run)
                // Fault transitions (allowed from any operational or startup state)
                | (Boot, Fault)  // Boot failures (e.g., hardware init failed)
                | (Init, Fault)
                | (PreOp, Fault)
                | (Run, Fault)
                // Safe stop from fault or run
                | (Fault, SafeStop)
                | (Run, SafeStop)
                // Recovery: fault -> init to retry
                | (Fault, Init)
                // Restart after safe stop
                | (SafeStop, Boot)
                // Direct stop from pre-op
                | (PreOp, SafeStop)
        )
    }

    /// Attempt to transition to `target`, returning error if invalid.
    pub fn transition_to(&mut self, target: RuntimeState) -> PlcResult<()> {
        if self.can_transition_to(target) {
            *self = target;
            Ok(())
        } else {
            Err(PlcError::InvalidStateTransition {
                from: self.to_string(),
                to: target.to_string(),
            })
        }
    }

    /// Returns true if the PLC is in an operational state.
    #[must_use]
    pub fn is_operational(&self) -> bool {
        matches!(self, Self::PreOp | Self::Run)
    }

    /// Returns true if the PLC is in a fault or stopped state.
    #[must_use]
    pub fn is_stopped(&self) -> bool {
        matches!(self, Self::Fault | Self::SafeStop)
    }
}

/// State machine wrapper with transition history tracking.
#[derive(Debug, Clone)]
pub struct StateMachine {
    current: RuntimeState,
    previous: Option<RuntimeState>,
    transition_count: u64,
}

impl Default for StateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl StateMachine {
    /// Create a new state machine starting in BOOT.
    #[must_use]
    pub fn new() -> Self {
        Self {
            current: RuntimeState::Boot,
            previous: None,
            transition_count: 0,
        }
    }

    /// Get the current state.
    #[must_use]
    pub fn state(&self) -> RuntimeState {
        self.current
    }

    /// Get the previous state (if any transition occurred).
    #[must_use]
    pub fn previous_state(&self) -> Option<RuntimeState> {
        self.previous
    }

    /// Get total number of transitions.
    #[must_use]
    pub fn transition_count(&self) -> u64 {
        self.transition_count
    }

    /// Attempt a state transition.
    pub fn transition(&mut self, target: RuntimeState) -> PlcResult<()> {
        if self.current.can_transition_to(target) {
            self.previous = Some(self.current);
            self.current = target;
            self.transition_count += 1;
            Ok(())
        } else {
            Err(PlcError::InvalidStateTransition {
                from: self.current.to_string(),
                to: target.to_string(),
            })
        }
    }

    /// Force a transition to FAULT state (always succeeds from operational states).
    pub fn enter_fault(&mut self) {
        if self.current.can_transition_to(RuntimeState::Fault) {
            self.previous = Some(self.current);
            self.current = RuntimeState::Fault;
            self.transition_count += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_forward_transitions() {
        let mut sm = StateMachine::new();
        assert_eq!(sm.state(), RuntimeState::Boot);

        assert!(sm.transition(RuntimeState::Init).is_ok());
        assert_eq!(sm.state(), RuntimeState::Init);

        assert!(sm.transition(RuntimeState::PreOp).is_ok());
        assert_eq!(sm.state(), RuntimeState::PreOp);

        assert!(sm.transition(RuntimeState::Run).is_ok());
        assert_eq!(sm.state(), RuntimeState::Run);
    }

    #[test]
    fn test_fault_transition() {
        let mut sm = StateMachine::new();
        sm.transition(RuntimeState::Init).unwrap();
        sm.transition(RuntimeState::PreOp).unwrap();
        sm.transition(RuntimeState::Run).unwrap();

        // Run -> Fault is valid
        assert!(sm.transition(RuntimeState::Fault).is_ok());
        assert_eq!(sm.state(), RuntimeState::Fault);

        // Fault -> SafeStop is valid
        assert!(sm.transition(RuntimeState::SafeStop).is_ok());
        assert_eq!(sm.state(), RuntimeState::SafeStop);
    }

    #[test]
    fn test_invalid_transition() {
        let mut sm = StateMachine::new();
        // Boot -> Run is invalid (must go through Init, PreOp)
        let result = sm.transition(RuntimeState::Run);
        assert!(result.is_err());
        assert_eq!(sm.state(), RuntimeState::Boot);
    }

    #[test]
    fn test_recovery_from_fault() {
        let mut sm = StateMachine::new();
        sm.transition(RuntimeState::Init).unwrap();
        sm.transition(RuntimeState::Fault).unwrap();

        // Fault -> Init is valid for recovery
        assert!(sm.transition(RuntimeState::Init).is_ok());
        assert_eq!(sm.state(), RuntimeState::Init);
    }

    #[test]
    fn test_transition_count() {
        let mut sm = StateMachine::new();
        assert_eq!(sm.transition_count(), 0);

        sm.transition(RuntimeState::Init).unwrap();
        assert_eq!(sm.transition_count(), 1);

        sm.transition(RuntimeState::PreOp).unwrap();
        assert_eq!(sm.transition_count(), 2);
    }

    #[test]
    fn test_enter_fault() {
        let mut sm = StateMachine::new();
        sm.transition(RuntimeState::Init).unwrap();
        sm.transition(RuntimeState::PreOp).unwrap();

        sm.enter_fault();
        assert_eq!(sm.state(), RuntimeState::Fault);
        assert_eq!(sm.previous_state(), Some(RuntimeState::PreOp));
    }

    #[test]
    fn test_boot_to_fault() {
        // Boot failures should be able to transition directly to Fault
        let mut sm = StateMachine::new();
        assert_eq!(sm.state(), RuntimeState::Boot);

        // Boot -> Fault is valid (e.g., hardware init failed)
        assert!(sm.transition(RuntimeState::Fault).is_ok());
        assert_eq!(sm.state(), RuntimeState::Fault);

        // Can recover from fault
        assert!(sm.transition(RuntimeState::Init).is_ok());
    }
}
