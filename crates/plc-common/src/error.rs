use thiserror::Error;

/// PLC error types covering configuration, runtime faults, and subsystem failures.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum PlcError {
    /// Configuration or initialization error.
    #[error("configuration error: {0}")]
    Config(String),

    /// Generic runtime fault.
    #[error("runtime fault: {0}")]
    Fault(String),

    /// Watchdog timer expired without being kicked.
    #[error("watchdog timeout: {0}")]
    WatchdogTimeout(String),

    /// Cycle execution exceeded the configured deadline.
    #[error("cycle overrun: expected {expected_ns}ns, actual {actual_ns}ns")]
    CycleOverrun {
        /// Expected cycle time in nanoseconds.
        expected_ns: u64,
        /// Actual cycle time in nanoseconds.
        actual_ns: u64,
    },

    /// Fieldbus communication or slave error.
    #[error("fieldbus error: {0}")]
    FieldbusError(String),

    /// EtherCAT working counter threshold exceeded.
    #[error("WKC threshold exceeded: {consecutive} consecutive errors (threshold: {threshold})")]
    WkcThresholdExceeded {
        /// Number of consecutive WKC errors.
        consecutive: u32,
        /// Configured threshold.
        threshold: u32,
    },

    /// WebAssembly trap or sandbox violation.
    #[error("wasm trap: {0}")]
    WasmTrap(String),

    /// I/O operation error.
    #[error("I/O error: {0}")]
    IoError(String),

    /// Invalid state transition attempted.
    #[error("invalid state transition from {from} to {to}")]
    InvalidStateTransition {
        /// Source state.
        from: String,
        /// Attempted target state.
        to: String,
    },
}

/// Convenience type alias for PLC operations.
pub type PlcResult<T> = Result<T, PlcError>;
