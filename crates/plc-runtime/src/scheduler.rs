//! Deterministic cyclic scheduler for the PLC runtime.
//!
//! The scheduler implements the classic PLC scan cycle:
//! 1. Read inputs from fieldbus
//! 2. Execute logic (Wasm module)
//! 3. Write outputs to fieldbus
//! 4. Wait for next cycle deadline
//!
//! Uses `clock_nanosleep` with `TIMER_ABSTIME` for jitter-free timing.

use crate::io_image::{IoImage, ProcessData};
use crate::wasm_host::LogicEngine;
use crate::watchdog::Watchdog;
use plc_common::config::{FaultPolicyConfig, OverrunPolicy, RuntimeConfig, SafeOutputPolicy};
use plc_common::error::{PlcError, PlcResult};
use plc_common::metrics::CycleMetrics;
use plc_common::state::{RuntimeState, StateMachine};
use std::time::{Duration, Instant};
use tracing::{debug, error, info, trace, warn};

/// Per-phase timing breakdown for diagnostics.
///
/// Provides granular visibility into where time is spent during each PLC cycle,
/// enabling identification of overrun causes (I/O vs logic execution).
#[derive(Debug, Clone, Copy, Default)]
pub struct CyclePhaseTimings {
    /// Time spent reading inputs from I/O image.
    pub io_read: Duration,
    /// Time spent executing Wasm logic.
    pub logic_exec: Duration,
    /// Time spent writing outputs to I/O image.
    pub io_write: Duration,
    /// Total cycle execution time (should equal io_read + logic_exec + io_write + overhead).
    pub total: Duration,
}

impl CyclePhaseTimings {
    /// Returns true if logic execution was the dominant phase.
    #[must_use]
    pub fn logic_dominant(&self) -> bool {
        self.logic_exec >= self.io_read && self.logic_exec >= self.io_write
    }

    /// Returns the overhead time not accounted for by the three phases.
    #[must_use]
    pub fn overhead(&self) -> Duration {
        self.total
            .saturating_sub(self.io_read)
            .saturating_sub(self.logic_exec)
            .saturating_sub(self.io_write)
    }
}

/// Result of a single cycle execution.
#[derive(Debug, Clone)]
pub struct CycleResult {
    /// Actual execution time of this cycle.
    pub execution_time: Duration,
    /// Whether the cycle exceeded the deadline.
    pub overrun: bool,
    /// Current cycle number.
    pub cycle_count: u64,
    /// Per-phase timing breakdown for diagnostics.
    pub phase_timings: CyclePhaseTimings,
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
    /// Fault handling policy.
    fault_policy: FaultPolicyConfig,
    /// Last known output values (for HoldLast safe output policy).
    last_outputs: ProcessData,
}

impl<E: LogicEngine> Scheduler<E> {
    /// Create a new scheduler with the given logic engine and configuration.
    pub fn new(engine: E, config: &RuntimeConfig) -> Self {
        let metrics = CycleMetrics::new(config.metrics.histogram_size, config.cycle_time);

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
            fault_policy: config.fault_policy.clone(),
            last_outputs: ProcessData::default(),
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
        self.engine
            .init()
            .map_err(|e| PlcError::Config(format!("Logic engine initialization failed: {e}")))?;

        // INIT → PRE_OP
        self.state.transition(RuntimeState::PreOp)?;

        info!("Scheduler initialized, state: PRE_OP");
        Ok(())
    }

    /// Start cyclic execution.
    ///
    /// Transitions from PRE_OP → RUN and starts the watchdog if configured.
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

        // Start the watchdog timer if configured
        if let Some(ref mut wd) = self.watchdog {
            wd.start(|| {
                // The callback is invoked when watchdog times out.
                // The triggered flag is already set by the watchdog itself.
                // The main loop checks watchdog_triggered() and enters fault state.
                error!("Watchdog triggered - RT loop has stopped responding");
            })?;
        }

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

        // Check if watchdog has triggered (RT loop was too slow)
        if self.watchdog_triggered() {
            self.enter_fault("Watchdog timeout detected")?;
            return Err(PlcError::Fault("Watchdog timeout".into()));
        }

        let cycle_start = Instant::now();

        // 1. Kick watchdog
        if let Some(ref wd) = self.watchdog {
            wd.kick();
        }

        // 2. Read inputs from I/O image (timed)
        let io_read_start = Instant::now();
        let inputs = self.io.read_inputs();
        let io_read_time = io_read_start.elapsed();

        // 3. Execute logic engine with inputs (timed)
        let logic_start = Instant::now();
        let outputs = match self.engine.step(&inputs) {
            Ok(outputs) => outputs,
            Err(e) => {
                self.enter_fault(&format!("Logic engine step failed: {e}"))?;
                return Err(e);
            }
        };
        let logic_exec_time = logic_start.elapsed();

        // 4. Write outputs to I/O image for fieldbus to read (timed)
        let io_write_start = Instant::now();
        // Only copy output fields, not the entire ProcessData
        self.io.write_outputs(|io_outputs| {
            io_outputs.digital_outputs = outputs.digital_outputs;
            io_outputs.analog_outputs = outputs.analog_outputs;
        });

        // Track last outputs for HoldLast safe output policy
        self.last_outputs.digital_outputs = outputs.digital_outputs;
        self.last_outputs.analog_outputs = outputs.analog_outputs;
        let io_write_time = io_write_start.elapsed();

        let execution_time = cycle_start.elapsed();
        let phase_timings = CyclePhaseTimings {
            io_read: io_read_time,
            logic_exec: logic_exec_time,
            io_write: io_write_time,
            total: execution_time,
        };
        self.cycle_count += 1;

        // 5. Record metrics
        self.metrics.record(execution_time);

        // 6. Check for overrun and apply fault policy
        let overrun = execution_time > self.cycle_period;
        if overrun {
            let overrun_amount = execution_time - self.cycle_period;
            if overrun_amount > self.max_overrun {
                // Critical overrun - apply overrun policy
                match self.fault_policy.on_overrun {
                    OverrunPolicy::Fault => {
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
                    OverrunPolicy::Warn => {
                        warn!(
                            cycle = self.cycle_count,
                            execution_us = execution_time.as_micros(),
                            deadline_us = self.cycle_period.as_micros(),
                            overrun_us = overrun_amount.as_micros(),
                            "Critical cycle overrun (policy: warn)"
                        );
                    }
                    OverrunPolicy::Ignore => {
                        trace!(
                            cycle = self.cycle_count,
                            overrun_us = overrun_amount.as_micros(),
                            "Critical cycle overrun ignored by policy"
                        );
                    }
                }
            } else {
                // Minor overrun within tolerance
                warn!(
                    cycle = self.cycle_count,
                    execution_us = execution_time.as_micros(),
                    deadline_us = self.cycle_period.as_micros(),
                    "Cycle overrun (within tolerance)"
                );
            }
        }

        // 7. Wait for next deadline
        if let Some(deadline) = self.next_deadline {
            self.wait_until(deadline);
            self.next_deadline = Some(deadline + self.cycle_period);
        }

        trace!(
            cycle = self.cycle_count,
            execution_us = execution_time.as_micros(),
            io_read_us = phase_timings.io_read.as_micros(),
            logic_exec_us = phase_timings.logic_exec.as_micros(),
            io_write_us = phase_timings.io_write.as_micros(),
            "Cycle complete"
        );

        Ok(CycleResult {
            execution_time,
            overrun,
            cycle_count: self.cycle_count,
            phase_timings,
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
    /// Transitions RUN → SAFE_STOP and stops the watchdog.
    pub fn stop(&mut self) -> PlcResult<()> {
        info!("Stopping scheduler");

        // Stop the watchdog first to prevent spurious triggers during shutdown
        if let Some(ref mut wd) = self.watchdog {
            wd.stop();
        }

        if self.state.state() == RuntimeState::Run {
            self.state.transition(RuntimeState::SafeStop)?;
        }

        // Set outputs to safe state
        self.set_safe_outputs();

        Ok(())
    }

    /// Enter fault state.
    ///
    /// This method is public to allow external components (e.g., the daemon)
    /// to trigger a fault state transition for external failures like fieldbus errors.
    pub fn enter_fault(&mut self, reason: &str) -> PlcResult<()> {
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

    /// Set outputs to safe values based on the configured safe output policy.
    ///
    /// Uses the seqlock-protected write_outputs() for thread-safe atomic update.
    fn set_safe_outputs(&mut self) {
        debug!(policy = ?self.fault_policy.safe_outputs, "Setting outputs to safe state");

        match &self.fault_policy.safe_outputs {
            SafeOutputPolicy::AllOff => {
                self.io.write_outputs(|outputs| {
                    outputs.digital_outputs = [0; 1];
                    outputs.analog_outputs = [0; 16];
                });
            }
            SafeOutputPolicy::HoldLast => {
                // Keep current outputs - they're already in place via last_outputs tracking
                // Just ensure the I/O image has the last known values
                let last = self.last_outputs;
                self.io.write_outputs(|outputs| {
                    outputs.digital_outputs = last.digital_outputs;
                    outputs.analog_outputs = last.analog_outputs;
                });
                debug!("Holding last output values");
            }
            SafeOutputPolicy::UserDefined { digital, analog } => {
                // Apply user-defined safe values
                let mut safe_digital = [0u32; 1];
                let mut safe_analog = [0i16; 16];

                // Copy user-defined values (up to array bounds)
                for (i, &val) in digital.iter().take(safe_digital.len()).enumerate() {
                    safe_digital[i] = val;
                }
                for (i, &val) in analog.iter().take(safe_analog.len()).enumerate() {
                    safe_analog[i] = val;
                }

                self.io.write_outputs(|outputs| {
                    outputs.digital_outputs = safe_digital;
                    outputs.analog_outputs = safe_analog;
                });
                debug!("Applied user-defined safe output values");
            }
        }
    }

    /// Wait until the specified deadline using high-precision absolute sleep.
    ///
    /// Uses `clock_nanosleep` with `TIMER_ABSTIME` to avoid jitter accumulation
    /// that would occur with relative sleep (drift between duration calculation
    /// and actual sleep call).
    #[cfg(target_os = "linux")]
    fn wait_until(&self, deadline: Instant) {
        // Convert Instant to timespec for clock_nanosleep
        let now = Instant::now();
        if deadline <= now {
            return; // Already past deadline
        }

        let duration = deadline - now;

        // Get current absolute time from CLOCK_MONOTONIC
        let mut current_ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };

        // SAFETY: clock_gettime is safe with valid clock_id and non-null pointer
        unsafe {
            libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut current_ts);
        }

        // Calculate absolute deadline by adding remaining duration to current time
        let deadline_nanos = current_ts.tv_nsec as u64 + duration.subsec_nanos() as u64;
        let deadline_ts = libc::timespec {
            tv_sec: current_ts.tv_sec
                + duration.as_secs() as libc::time_t
                + (deadline_nanos / 1_000_000_000) as libc::time_t,
            tv_nsec: (deadline_nanos % 1_000_000_000) as libc::c_long,
        };

        // SAFETY: clock_nanosleep is safe with valid parameters
        // Using TIMER_ABSTIME (1) for absolute timing to prevent jitter accumulation
        // Loop to handle EINTR (signal interruption) - retry until deadline passes
        loop {
            let ret = unsafe {
                libc::clock_nanosleep(
                    libc::CLOCK_MONOTONIC,
                    libc::TIMER_ABSTIME,
                    &deadline_ts,
                    std::ptr::null_mut(),
                )
            };
            // ret == 0 means success; anything other than EINTR is also exit
            if ret == 0 || ret != libc::EINTR {
                break;
            }
            // On EINTR, check if we've passed the deadline before retrying
            if Instant::now() >= deadline {
                break;
            }
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
        self.watchdog.as_ref().is_some_and(|wd| wd.has_triggered())
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

        fn step(
            &mut self,
            inputs: &crate::io_image::ProcessData,
        ) -> PlcResult<crate::io_image::ProcessData> {
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

    #[test]
    fn test_phase_timings() {
        let engine = MockEngine::new();
        let config = RuntimeConfig {
            cycle_time: Duration::from_millis(10),
            ..Default::default()
        };
        let mut scheduler = Scheduler::new(engine, &config);

        scheduler.initialize().unwrap();
        scheduler.start().unwrap();

        let result = scheduler.run_cycle().unwrap();

        // All phase timings should be non-negative and non-zero for total
        assert!(result.phase_timings.total > Duration::ZERO);

        // Total should be >= sum of individual phases
        let sum = result.phase_timings.io_read
            + result.phase_timings.logic_exec
            + result.phase_timings.io_write;
        assert!(result.phase_timings.total >= sum);

        // Overhead should be small (< 1ms for a simple mock)
        assert!(result.phase_timings.overhead() < Duration::from_millis(1));
    }

    #[test]
    fn test_cycle_phase_timings_methods() {
        // Test logic_dominant when logic is longest
        let timings = CyclePhaseTimings {
            io_read: Duration::from_micros(10),
            logic_exec: Duration::from_micros(100),
            io_write: Duration::from_micros(10),
            total: Duration::from_micros(125),
        };
        assert!(timings.logic_dominant());
        assert_eq!(timings.overhead(), Duration::from_micros(5));

        // Test logic_dominant when IO is longer
        let timings2 = CyclePhaseTimings {
            io_read: Duration::from_micros(100),
            logic_exec: Duration::from_micros(10),
            io_write: Duration::from_micros(10),
            total: Duration::from_micros(120),
        };
        assert!(!timings2.logic_dominant());
    }
}
