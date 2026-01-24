//! Watchdog timer for fault detection.
//!
//! Provides both software and hardware watchdog support:
//!
//! - Software watchdog: A separate thread monitors the RT loop
//! - Hardware watchdog: Uses `/dev/watchdog` on Linux for system-level protection
//!
//! The RT loop must "kick" the watchdog each cycle. If the watchdog
//! is not kicked within the timeout period, a fault is triggered.

use plc_common::error::{PlcError, PlcResult};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

/// Watchdog timer that monitors the RT loop.
#[derive(Debug)]
pub struct Watchdog {
    /// Shared state between the RT loop and watchdog thread.
    state: Arc<WatchdogState>,
    /// Handle to the watchdog monitor thread.
    monitor_handle: Option<JoinHandle<()>>,
    /// Configured timeout duration.
    timeout: Duration,
    /// Whether the watchdog is currently running.
    running: Arc<AtomicBool>,
}

/// Shared state for watchdog synchronization.
#[derive(Debug)]
struct WatchdogState {
    /// Timestamp of last kick (nanoseconds since start).
    last_kick_ns: AtomicU64,
    /// Monotonic start time for relative timestamps.
    start_time: Instant,
    /// Flag set when watchdog triggers.
    triggered: AtomicBool,
    /// Flag to signal monitor thread to stop.
    stop_requested: AtomicBool,
}

impl WatchdogState {
    fn new() -> Self {
        Self {
            last_kick_ns: AtomicU64::new(0),
            start_time: Instant::now(),
            triggered: AtomicBool::new(false),
            stop_requested: AtomicBool::new(false),
        }
    }

    /// Get elapsed nanoseconds since start.
    fn elapsed_ns(&self) -> u64 {
        self.start_time.elapsed().as_nanos() as u64
    }

    /// Record a kick.
    fn kick(&self) {
        self.last_kick_ns.store(self.elapsed_ns(), Ordering::Release);
    }

    /// Check if watchdog has timed out.
    fn is_timed_out(&self, timeout_ns: u64) -> bool {
        let last = self.last_kick_ns.load(Ordering::Acquire);
        let now = self.elapsed_ns();
        now.saturating_sub(last) > timeout_ns
    }
}

impl Watchdog {
    /// Create a new watchdog with the specified timeout.
    ///
    /// The watchdog is created in a stopped state. Call `start()` to begin monitoring.
    pub fn new(timeout: Duration) -> Self {
        Self {
            state: Arc::new(WatchdogState::new()),
            monitor_handle: None,
            timeout,
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Start the watchdog monitor thread.
    ///
    /// The callback is invoked when the watchdog triggers.
    pub fn start<F>(&mut self, on_trigger: F) -> PlcResult<()>
    where
        F: Fn() + Send + 'static,
    {
        if self.running.load(Ordering::Acquire) {
            return Err(PlcError::Config("Watchdog already running".into()));
        }

        info!(timeout_ms = self.timeout.as_millis(), "Starting watchdog");

        // Clear flags from any previous run to allow restart
        self.state.stop_requested.store(false, Ordering::Release);
        self.state.triggered.store(false, Ordering::Release);

        // Initial kick to set baseline
        self.state.kick();

        let state = Arc::clone(&self.state);
        let running = Arc::clone(&self.running);
        let timeout_ns = self.timeout.as_nanos() as u64;
        let check_interval = self.timeout / 4; // Check 4x per timeout period

        // Clamp check_interval to minimum 1ms to avoid spin
        let check_interval = check_interval.max(Duration::from_millis(1));

        // Set running BEFORE spawn so is_running() returns true immediately
        self.running.store(true, Ordering::Release);

        let handle = match thread::Builder::new()
            .name("plc-watchdog".into())
            .spawn(move || {
                debug!("Watchdog monitor thread started");

                while !state.stop_requested.load(Ordering::Acquire) {
                    thread::sleep(check_interval);

                    if state.stop_requested.load(Ordering::Acquire) {
                        break;
                    }

                    if state.is_timed_out(timeout_ns) {
                        if !state.triggered.swap(true, Ordering::AcqRel) {
                            error!("Watchdog timeout! RT loop has not responded.");
                            on_trigger();
                        }
                    }
                }

                running.store(false, Ordering::Release);
                debug!("Watchdog monitor thread stopped");
            }) {
            Ok(h) => h,
            Err(e) => {
                // Reset running flag on spawn failure
                self.running.store(false, Ordering::Release);
                return Err(PlcError::Config(format!(
                    "Failed to spawn watchdog thread: {e}"
                )));
            }
        };

        self.monitor_handle = Some(handle);
        Ok(())
    }

    /// Kick the watchdog to indicate the RT loop is alive.
    ///
    /// This should be called once per scan cycle.
    #[inline]
    pub fn kick(&self) {
        self.state.kick();
    }

    /// Check if the watchdog has triggered.
    #[inline]
    pub fn has_triggered(&self) -> bool {
        self.state.triggered.load(Ordering::Acquire)
    }

    /// Reset the watchdog state after a fault has been handled.
    ///
    /// This clears the triggered flag and kicks the watchdog.
    /// Use this when the watchdog is still running but you want to
    /// acknowledge a trigger and continue.
    pub fn reset(&self) {
        self.state.triggered.store(false, Ordering::Release);
        self.state.kick();
        info!("Watchdog reset");
    }

    /// Fully reset the watchdog state for restart.
    ///
    /// This clears all flags (stop_requested, triggered, running).
    /// Use this before calling `start()` after a `stop()`.
    pub fn full_reset(&mut self) {
        self.state.stop_requested.store(false, Ordering::Release);
        self.state.triggered.store(false, Ordering::Release);
        self.running.store(false, Ordering::Release);
        self.state.kick();
        info!("Watchdog full reset");
    }

    /// Stop and restart the watchdog.
    ///
    /// Convenience method that calls `stop()` then `start()`.
    pub fn restart<F>(&mut self, on_trigger: F) -> PlcResult<()>
    where
        F: Fn() + Send + 'static,
    {
        self.stop();
        self.start(on_trigger)
    }

    /// Stop the watchdog monitor thread.
    pub fn stop(&mut self) {
        if !self.running.load(Ordering::Acquire) {
            return;
        }

        info!("Stopping watchdog");
        self.state.stop_requested.store(true, Ordering::Release);

        if let Some(handle) = self.monitor_handle.take() {
            // Wake up the thread if it's sleeping
            // (it will check stop_requested on next iteration)
            if let Err(e) = handle.join() {
                warn!("Watchdog thread panicked: {:?}", e);
            }
        }
    }

    /// Check if the watchdog is running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    /// Get time since last kick.
    pub fn time_since_kick(&self) -> Duration {
        let last = self.state.last_kick_ns.load(Ordering::Acquire);
        let now = self.state.elapsed_ns();
        Duration::from_nanos(now.saturating_sub(last))
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Hardware watchdog interface (Linux `/dev/watchdog`).
///
/// This provides a second layer of protection: if the entire process
/// hangs or crashes, the hardware will trigger a system reset.
#[cfg(target_os = "linux")]
pub struct HardwareWatchdog {
    fd: std::os::unix::io::RawFd,
    timeout_secs: u32,
}

#[cfg(target_os = "linux")]
impl HardwareWatchdog {
    /// Open the hardware watchdog device.
    ///
    /// # Safety
    ///
    /// Opening `/dev/watchdog` starts the hardware timer. If you don't
    /// kick it regularly, the system will reset!
    pub fn open(timeout_secs: u32) -> PlcResult<Self> {
        use std::os::unix::io::IntoRawFd;

        let file = std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/watchdog")
            .map_err(|e| PlcError::IoError(format!("Failed to open /dev/watchdog: {e}")))?;

        let fd = file.into_raw_fd();

        // Set timeout using ioctl
        // WDIOC_SETTIMEOUT = 0xC0045706
        const WDIOC_SETTIMEOUT: libc::c_ulong = 0xC004_5706;
        let mut timeout = timeout_secs as libc::c_int;

        let result = unsafe { libc::ioctl(fd, WDIOC_SETTIMEOUT, &mut timeout) };
        if result < 0 {
            // Close fd before returning error
            unsafe { libc::close(fd) };
            return Err(PlcError::IoError(format!(
                "Failed to set watchdog timeout: {}",
                std::io::Error::last_os_error()
            )));
        }

        info!(
            timeout_secs,
            actual_timeout = timeout, "Hardware watchdog opened"
        );

        Ok(Self {
            fd,
            timeout_secs: timeout as u32,
        })
    }

    /// Kick the hardware watchdog.
    pub fn kick(&self) -> PlcResult<()> {
        // Write any byte to kick the watchdog
        let result = unsafe { libc::write(self.fd, b"k".as_ptr() as *const libc::c_void, 1) };
        if result < 0 {
            return Err(PlcError::IoError(format!(
                "Failed to kick hardware watchdog: {}",
                std::io::Error::last_os_error()
            )));
        }
        Ok(())
    }

    /// Get the configured timeout.
    pub fn timeout_secs(&self) -> u32 {
        self.timeout_secs
    }

    /// Disable the hardware watchdog (write 'V' magic close).
    pub fn disable(self) -> PlcResult<()> {
        // Write magic 'V' to disable watchdog on close
        let result = unsafe { libc::write(self.fd, b"V".as_ptr() as *const libc::c_void, 1) };
        if result < 0 {
            warn!(
                "Failed to send magic close to hardware watchdog: {}",
                std::io::Error::last_os_error()
            );
        }
        // fd will be closed by Drop
        Ok(())
    }
}

#[cfg(target_os = "linux")]
impl Drop for HardwareWatchdog {
    fn drop(&mut self) {
        // Close the file descriptor
        // Note: This will cause a system reset if magic close wasn't sent!
        unsafe { libc::close(self.fd) };
    }
}

/// Placeholder for non-Linux systems.
#[cfg(not(target_os = "linux"))]
pub struct HardwareWatchdog {
    _private: (),
}

#[cfg(not(target_os = "linux"))]
impl HardwareWatchdog {
    /// Hardware watchdog not available on this platform.
    pub fn open(_timeout_secs: u32) -> PlcResult<Self> {
        Err(PlcError::Config(
            "Hardware watchdog not available on this platform".into(),
        ))
    }

    /// No-op on non-Linux.
    pub fn kick(&self) -> PlcResult<()> {
        Ok(())
    }

    /// No-op on non-Linux.
    pub fn timeout_secs(&self) -> u32 {
        0
    }

    /// No-op on non-Linux.
    pub fn disable(self) -> PlcResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn test_watchdog_kick() {
        let wd = Watchdog::new(Duration::from_millis(100));
        wd.kick();
        assert!(!wd.has_triggered());

        let elapsed = wd.time_since_kick();
        assert!(elapsed < Duration::from_millis(10));
    }

    #[test]
    fn test_watchdog_state_timeout() {
        let state = WatchdogState::new();
        state.kick();

        // Should not be timed out immediately
        assert!(!state.is_timed_out(1_000_000_000)); // 1 second

        // Sleep a bit and check again
        std::thread::sleep(Duration::from_millis(10));
        assert!(state.is_timed_out(1_000_000)); // 1ms timeout should trigger
    }

    #[test]
    fn test_watchdog_trigger_callback() {
        let trigger_count = Arc::new(AtomicUsize::new(0));
        let trigger_count_clone = Arc::clone(&trigger_count);

        let mut wd = Watchdog::new(Duration::from_millis(50));
        wd.start(move || {
            trigger_count_clone.fetch_add(1, Ordering::Relaxed);
        })
        .unwrap();

        // Don't kick - let it timeout
        std::thread::sleep(Duration::from_millis(200));

        assert!(wd.has_triggered());
        assert!(trigger_count.load(Ordering::Relaxed) >= 1);

        wd.stop();
    }

    #[test]
    fn test_watchdog_no_trigger_with_kicks() {
        let trigger_count = Arc::new(AtomicUsize::new(0));
        let trigger_count_clone = Arc::clone(&trigger_count);

        let mut wd = Watchdog::new(Duration::from_millis(100));
        wd.start(move || {
            trigger_count_clone.fetch_add(1, Ordering::Relaxed);
        })
        .unwrap();

        // Keep kicking
        for _ in 0..10 {
            wd.kick();
            std::thread::sleep(Duration::from_millis(20));
        }

        assert!(!wd.has_triggered());
        assert_eq!(trigger_count.load(Ordering::Relaxed), 0);

        wd.stop();
    }

    #[test]
    fn test_watchdog_reset() {
        let mut wd = Watchdog::new(Duration::from_millis(50));
        wd.start(|| {}).unwrap();

        // Let it timeout
        std::thread::sleep(Duration::from_millis(150));
        assert!(wd.has_triggered());

        // Reset clears the flag and kicks
        wd.reset();
        // Check immediately after reset (before watchdog can re-trigger)
        assert!(!wd.has_triggered());

        // Keep kicking to prevent re-trigger
        wd.kick();

        wd.stop();
    }

    #[test]
    fn test_watchdog_restart() {
        let trigger_count = Arc::new(AtomicUsize::new(0));
        let trigger_count_clone = Arc::clone(&trigger_count);

        let mut wd = Watchdog::new(Duration::from_millis(100));

        // Start and kick
        wd.start(move || {
            trigger_count_clone.fetch_add(1, Ordering::Relaxed);
        })
        .unwrap();
        wd.kick();
        assert!(wd.is_running());

        // Stop
        wd.stop();
        // Wait for thread to actually stop
        std::thread::sleep(Duration::from_millis(50));
        assert!(!wd.is_running());

        // Restart should work
        let trigger_count_clone2 = Arc::clone(&trigger_count);
        wd.restart(move || {
            trigger_count_clone2.fetch_add(1, Ordering::Relaxed);
        })
        .unwrap();
        assert!(wd.is_running());

        // Keep kicking - should not trigger
        for _ in 0..5 {
            wd.kick();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!wd.has_triggered());

        wd.stop();
    }
}
