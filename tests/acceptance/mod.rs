//! Integration tests for Virtual PLC acceptance testing.
//!
//! These tests verify real-time performance characteristics:
//! - Latency and jitter under load
//! - Watchdog fault detection and recovery
//! - Long-duration stability (soak tests)
//!
//! Most tests require:
//! - Root privileges
//! - PREEMPT_RT kernel (recommended)
//! - cyclictest (rt-tests package)
//! - stress-ng (for load generation)

mod common;
mod latency_test;
mod soak_test;
mod watchdog_test;
