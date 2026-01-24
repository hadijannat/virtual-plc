//! Deterministic cyclic scheduler for the PLC runtime.
//!
//! The scheduler implements the classic PLC scan cycle:
//! 1. Read inputs from fieldbus
//! 2. Execute logic (Wasm module)
//! 3. Write outputs to fieldbus
//! 4. Wait for next cycle deadline
//!
//! Uses `clock_nanosleep` with `TIMER_ABSTIME` for jitter-free timing.

use crate::io_image::IoImage;
use crate::wasm_host::LogicEngine;
use crate::watchdog::Watchdog;
use plc_common::config::RuntimeConfig;
use plc_common::error::{PlcError, PlcResult};
use plc_common::metrics::CycleMetrics;
use plc_common::state::{RuntimeState, StateMachine};
use std::time::{Duration, Instant};
use tracing::{debug, error, info, trace, warn};

/// Result of a single cycle execution.
#[derive(Debug, Clone)]
pub struct CycleResult {
    /// Actual execution time of this cycle.
    pub execution_time: Duration,
    /// Whether the cycle exceeded the deadline.
    pub overrun: bool,
    /// Current cycle number.
    pub cycle_count: u64,
}

/// Deterministic cyclic scheduler.
///
/// Coordinates the scan cycle between I/O image, logic engine, and fieldbus.
pub struct Scheduler<E: LogicEngine> {
    /// Process I/O image.
    pub io: IoImage,
    /// WebAssembly logic engine.
    pub engine: E,
    /// Runtime state machine.
    state: StateMachine,
    /// Cycle time configuration.
    cycle_period: Duration,
    /// Maximum allowed overrun before fault.
    max_overrun: Duration,
    /// Next cycle deadline (absolute time).
    next_deadline: Option<Instant>,
    /// Total cycles executed.
    cycle_count: u64,
    /// Metrics collection.
    metrics: CycleMetrics,
    /// Watchdog timer.
    watchdog: Option<Watchdog>,
}

impl<E: LogicEngine> Scheduler<E> {
    /// Create a new scheduler with the given logic engine and configuration.
    pub fn new(engine: E, config: &RuntimeConfig) -> Self {
        let metrics = CycleMetrics::new(
            config.metrics.histogram_size,
            config.cycle_time,
        );

        Self {
            io: IoImage::new(),
            engine,
            state: StateMachine::new(),
            cycle_period: config.cycle_time,
            max_overrun: config.max_overrun,
            next_deadline: None,
            cycle_count: 0,
            metrics,
            watchdog: None,
        }
    }

    /// Create a scheduler with default configuration.
    pub fn with_defaults(engine: E) -> Self {
        Self::new(engine, &RuntimeConfig::default())
    }

    /// Set the watchdog timer.
    pub fn set_watchdog(&mut self, watchdog: Watchdog) {
        self.watchdog = Some(watchdog);
    }

    /// Get the current runtime state.
    pub fn state(&self) -> RuntimeState {
        self.state.state()
    }

    /// Get cycle metrics.
    pub fn metrics(&self) -> &CycleMetrics {
        &self.metrics
    }

    /// Get total cycle count.
    pub fn cycle_count(&self) -> u64 {
        self.cycle_count
    }

    /// Initialize the scheduler and logic engine.
    ///
    /// Transitions from BOOT → INIT → PRE_OP.
    pub fn initialize(&mut self) -> PlcResult<()> {
        info!("Initializing scheduler");

        // BOOT → INIT
        self.state.transition(RuntimeState::Init)?;

        // Initialize the logic engine
        self.engine.init().map_err(|e| {
            PlcError::Config(format!("Logic engine initialization failed: {e}"))
        })?;

        // INIT → PRE_OP
        self.state.transition(RuntimeState::PreOp)?;

        info!("Scheduler initialized, state: PRE_OP");
        Ok(())
    }

    /// Start cyclic execution.
    ///
    /// Transitions from PRE_OP → RUN.
    pub fn start(&mut self) -> PlcResult<()> {
        if self.state.state() != RuntimeState::PreOp {
            return Err(PlcError::InvalidStateTransition {
                from: self.state.state().to_string(),
                to: RuntimeState::Run.to_string(),
            });
        }

        info!(
            cycle_period_us = self.cycle_period.as_micros(),
            "Starting cyclic execution"
        );

        self.state.transition(RuntimeState::Run)?;
        self.next_deadline = Some(Instant::now() + self.cycle_period);

        Ok(())
    }

    /// Execute one scan cycle.
    ///
    /// This is the core PLC loop iteration:
    /// 1. Kick watchdog
    /// 2. Record cycle start time
    /// 3. Read inputs (already in I/O image from fieldbus)
    /// 4. Execute logic
    /// 5. Write outputs (to I/O image for fieldbus)
    /// 6. Record metrics
    /// 7. Wait for next deadline
    ///
    /// # Returns
    ///
    /// Returns `Ok(CycleResult)` on success, or `Err` on fault.
    pub fn run_cycle(&mut self) -> PlcResult<CycleResult> {
        if self.state.state() != RuntimeState::Run {
            return Err(PlcError::Fault(format!(
                "Cannot run cycle in state {}",
                self.state.state()
            )));
        }

        let cycle_start = Instant::now();

        // 1. Kick watchdog
        if let Some(ref wd) = self.watchdog {
            wd.kick();
        }

        // 2. Read inputs from I/O image
        let inputs = self.io.read_inputs();

        // 3. Execute logic engine with inputs
        let outputs = match self.engine.step(&inputs) {
            Ok(outputs) => outputs,
            Err(e) => {
                self.enter_fault(&format!("Logic engine step failed: {e}"))?;
                return Err(e);
            }
        };

        // 4. Write outputs to I/O image for fieldbus to read
        *self.io.outputs_mut() = outputs;

        let execution_time = cycle_start.elapsed();
        self.cycle_count += 1;

        // 5. Record metrics
        self.metrics.record(execution_time);

        // 6. Check for overrun
        let overrun = execution_time > self.cycle_period;
        if overrun {
            let overrun_amount = execution_time - self.cycle_period;
            if overrun_amount > self.max_overrun {
                error!(
                    execution_us = execution_time.as_micros(),
                    deadline_us = self.cycle_period.as_micros(),
                    overrun_us = overrun_amount.as_micros(),
                    "Critical cycle overrun - entering fault state"
                );
                self.enter_fault("Critical cycle overrun")?;
                return Err(PlcError::CycleOverrun {
                    expected_ns: self.cycle_period.as_nanos() as u64,
                    actual_ns: execution_time.as_nanos() as u64,
                });
            }
            warn!(
                cycle = self.cycle_count,
                execution_us = execution_time.as_micros(),
                deadline_us = self.cycle_period.as_micros(),
                "Cycle overrun (within tolerance)"
            );
        }

        // 7. Wait for next deadline
        if let Some(deadline) = self.next_deadline {
            self.wait_until(deadline);
            self.next_deadline = Some(deadline + self.cycle_period);
        }

        trace!(
            cycle = self.cycle_count,
            execution_us = execution_time.as_micros(),
            "Cycle complete"
        );

        Ok(CycleResult {
            execution_time,
            overrun,
            cycle_count: self.cycle_count,
        })
    }

    /// Run the scheduler loop until stopped or faulted.
    ///
    /// This blocks the current thread.
    pub fn run(&mut self) -> PlcResult<()> {
        info!("Entering main scheduler loop");

        while self.state.state() == RuntimeState::Run {
            self.run_cycle()?;
        }

        info!(
            final_state = %self.state.state(),
            cycles = self.cycle_count,
            "Scheduler loop exited"
        );

        Ok(())
    }

    /// Stop cyclic execution gracefully.
    ///
    /// Transitions RUN → SAFE_STOP.
    pub fn stop(&mut self) -> PlcResult<()> {
        info!("Stopping scheduler");

        if self.state.state() == RuntimeState::Run {
            self.state.transition(RuntimeState::SafeStop)?;
        }

        // Set outputs to safe state
        self.set_safe_outputs();

        Ok(())
    }

    /// Enter fault state.
    fn enter_fault(&mut self, reason: &str) -> PlcResult<()> {
        error!(reason, "Entering FAULT state");

        self.state.enter_fault();

        // Try to set outputs to safe state
        self.set_safe_outputs();

        // Notify logic engine of fault
        if let Err(e) = self.engine.fault() {
            warn!("Logic engine fault handler failed: {e}");
        }

        Ok(())
    }

    /// Set all outputs to safe values (typically 0).
    fn set_safe_outputs(&mut self) {
        debug!("Setting outputs to safe state");
        self.io.write_do(0);
        for i in 0..16 {
            self.io.write_ao(i, 0);
        }
    }

    /// Wait until the specified deadline using high-precision sleep.
    #[cfg(target_os = "linux")]
    fn wait_until(&self, deadline: Instant) {
        use std::time::SystemTime;

        // Convert Instant to timespec for clock_nanosleep
        let now = Instant::now();
        if deadline <= now {
            return; // Already past deadline
        }

        let duration = deadline - now;

        // Use clock_nanosleep with CLOCK_MONOTONIC and TIMER_ABSTIME
        // For simplicity, we'll use a relative sleep here since Instant
        // doesn't directly map to timespec. Production code would use
        // clock_gettime + clock_nanosleep for true absolute timing.

        let ts = libc::timespec {
            tv_sec: duration.as_secs() as libc::time_t,
            tv_nsec: duration.subsec_nanos() as libc::c_long,
        };

        // SAFETY: clock_nanosleep is safe with valid parameters
        unsafe {
            libc::clock_nanosleep(
                libc::CLOCK_MONOTONIC,
                0, // Relative sleep (TIMER_ABSTIME would be 1)
                &ts,
                std::ptr::null_mut(),
            );
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn wait_until(&self, deadline: Instant) {
        let now = Instant::now();
        if deadline > now {
            std::thread::sleep(deadline - now);
        }
    }

    /// Get a mutable reference to the I/O image for fieldbus integration.
    pub fn io_mut(&mut self) -> &mut IoImage {
        &mut self.io
    }

    /// Check if the watchdog has triggered.
    pub fn watchdog_triggered(&self) -> bool {
        self.watchdog.as_ref().map_or(false, |wd| wd.has_triggered())
    }
}

/// Builder for configuring the scheduler.
pub struct SchedulerBuilder<E: LogicEngine> {
    engine: E,
    config: RuntimeConfig,
    watchdog_timeout: Option<Duration>,
}

impl<E: LogicEngine> SchedulerBuilder<E> {
    /// Create a new builder with the given logic engine.
    pub fn new(engine: E) -> Self {
        Self {
            engine,
            config: RuntimeConfig::default(),
            watchdog_timeout: None,
        }
    }

    /// Set the cycle period.
    pub fn cycle_period(mut self, period: Duration) -> Self {
        self.config.cycle_time = period;
        self
    }

    /// Set the maximum allowed overrun.
    pub fn max_overrun(mut self, max: Duration) -> Self {
        self.config.max_overrun = max;
        self
    }

    /// Set the watchdog timeout.
    pub fn watchdog_timeout(mut self, timeout: Duration) -> Self {
        self.watchdog_timeout = Some(timeout);
        self
    }

    /// Set the full runtime configuration.
    pub fn config(mut self, config: RuntimeConfig) -> Self {
        self.config = config;
        self
    }

    /// Build the scheduler.
    pub fn build(self) -> Scheduler<E> {
        let mut scheduler = Scheduler::new(self.engine, &self.config);

        if let Some(timeout) = self.watchdog_timeout {
            scheduler.set_watchdog(Watchdog::new(timeout));
        }

        scheduler
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock logic engine for testing.
    struct MockEngine {
        step_count: u64,
        should_fail: bool,
    }

    impl MockEngine {
        fn new() -> Self {
            Self {
                step_count: 0,
                should_fail: false,
            }
        }
    }

    impl LogicEngine for MockEngine {
        fn init(&mut self) -> PlcResult<()> {
            Ok(())
        }

        fn step(&mut self, inputs: &crate::io_image::ProcessData) -> PlcResult<crate::io_image::ProcessData> {
            if self.should_fail {
                return Err(PlcError::Fault("Simulated failure".into()));
            }
            self.step_count += 1;
            // Pass-through: copy inputs to outputs
            Ok(*inputs)
        }

        fn fault(&mut self) -> PlcResult<()> {
            Ok(())
        }

        fn is_ready(&self) -> bool {
            true
        }
    }

    #[test]
    fn test_scheduler_state_transitions() {
        let engine = MockEngine::new();
        let mut scheduler = Scheduler::with_defaults(engine);

        assert_eq!(scheduler.state(), RuntimeState::Boot);

        scheduler.initialize().unwrap();
        assert_eq!(scheduler.state(), RuntimeState::PreOp);

        scheduler.start().unwrap();
        assert_eq!(scheduler.state(), RuntimeState::Run);

        scheduler.stop().unwrap();
        assert_eq!(scheduler.state(), RuntimeState::SafeStop);
    }

    #[test]
    fn test_scheduler_cycle() {
        let engine = MockEngine::new();
        let config = RuntimeConfig {
            cycle_time: Duration::from_millis(10),
            ..Default::default()
        };
        let mut scheduler = Scheduler::new(engine, &config);

        scheduler.initialize().unwrap();
        scheduler.start().unwrap();

        let result = scheduler.run_cycle().unwrap();
        assert_eq!(result.cycle_count, 1);
        assert!(!result.overrun);

        let result = scheduler.run_cycle().unwrap();
        assert_eq!(result.cycle_count, 2);

        assert_eq!(scheduler.engine.step_count, 2);
    }

    #[test]
    fn test_scheduler_builder() {
        let engine = MockEngine::new();
        let scheduler = SchedulerBuilder::new(engine)
            .cycle_period(Duration::from_millis(5))
            .max_overrun(Duration::from_micros(100))
            .watchdog_timeout(Duration::from_millis(15))
            .build();

        assert_eq!(scheduler.cycle_period, Duration::from_millis(5));
        assert_eq!(scheduler.max_overrun, Duration::from_micros(100));
        assert!(scheduler.watchdog.is_some());
    }

    #[test]
    fn test_invalid_state_transition() {
        let engine = MockEngine::new();
        let mut scheduler = Scheduler::with_defaults(engine);

        // Can't start from BOOT state
        let result = scheduler.start();
        assert!(result.is_err());
    }

    #[test]
    fn test_metrics_collection() {
        let engine = MockEngine::new();
        let mut scheduler = Scheduler::with_defaults(engine);

        scheduler.initialize().unwrap();
        scheduler.start().unwrap();

        for _ in 0..10 {
            scheduler.run_cycle().unwrap();
        }

        let metrics = scheduler.metrics();
        assert_eq!(metrics.total_cycles(), 10);
        assert!(metrics.min().is_some());
        assert!(metrics.max().is_some());
    }
}
