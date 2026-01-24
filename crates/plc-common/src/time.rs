/// Monotonic time helpers (scaffold).
///
/// In production: use `clock_gettime(CLOCK_MONOTONIC_RAW)` and define a stable timebase
/// shared between runtime + fieldbus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CycleIndex(pub u64);
