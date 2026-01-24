//! Signal handling for graceful daemon shutdown.
//!
//! Provides Unix signal handling (SIGTERM, SIGINT, SIGHUP) for clean
//! shutdown of the PLC daemon. Uses atomic flags to communicate
//! shutdown requests to the main loop without blocking.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use tracing::{debug, info};

/// Signal types that the daemon handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalKind {
    /// SIGTERM - Graceful termination request.
    Terminate,
    /// SIGINT - Interrupt (Ctrl+C).
    Interrupt,
    /// SIGHUP - Hangup, often used for config reload.
    Hangup,
}

impl std::fmt::Display for SignalKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignalKind::Terminate => write!(f, "SIGTERM"),
            SignalKind::Interrupt => write!(f, "SIGINT"),
            SignalKind::Hangup => write!(f, "SIGHUP"),
        }
    }
}

/// Shared state for signal handling.
///
/// This struct is shared between the signal handler and the main loop.
/// All fields use atomic operations for thread-safe access.
#[derive(Debug)]
pub struct SignalState {
    /// Set to true when a shutdown signal is received.
    shutdown_requested: AtomicBool,
    /// Set to true when a reload signal is received.
    reload_requested: AtomicBool,
    /// Count of signals received (for diagnostics).
    signal_count: AtomicU32,
    /// The most recent signal received.
    last_signal: AtomicU32,
}

impl Default for SignalState {
    fn default() -> Self {
        Self::new()
    }
}

impl SignalState {
    /// Create a new signal state.
    pub fn new() -> Self {
        Self {
            shutdown_requested: AtomicBool::new(false),
            reload_requested: AtomicBool::new(false),
            signal_count: AtomicU32::new(0),
            last_signal: AtomicU32::new(0),
        }
    }

    /// Check if shutdown has been requested.
    #[inline]
    pub fn shutdown_requested(&self) -> bool {
        self.shutdown_requested.load(Ordering::Relaxed)
    }

    /// Check if reload has been requested (and clear the flag).
    #[inline]
    pub fn take_reload_request(&self) -> bool {
        self.reload_requested.swap(false, Ordering::Relaxed)
    }

    /// Request shutdown (can be called from any thread).
    pub fn request_shutdown(&self) {
        self.shutdown_requested.store(true, Ordering::Relaxed);
    }

    /// Request reload (can be called from any thread).
    pub fn request_reload(&self) {
        self.reload_requested.store(true, Ordering::Relaxed);
    }

    /// Record a signal.
    fn record_signal(&self, kind: SignalKind) {
        self.signal_count.fetch_add(1, Ordering::Relaxed);
        self.last_signal.store(kind as u32, Ordering::Relaxed);
    }

    /// Get the total number of signals received.
    pub fn signal_count(&self) -> u32 {
        self.signal_count.load(Ordering::Relaxed)
    }
}

/// Handle for signal management.
///
/// Holds the shared state and provides methods to check for signals.
#[derive(Clone)]
pub struct SignalHandler {
    state: Arc<SignalState>,
}

impl SignalHandler {
    /// Create a new signal handler and register signal handlers.
    ///
    /// On Unix systems, this registers handlers for SIGTERM, SIGINT, and SIGHUP.
    /// On other platforms, this creates a handler that only supports manual shutdown.
    pub fn new() -> std::io::Result<Self> {
        let state = Arc::new(SignalState::new());
        let handler = Self {
            state: Arc::clone(&state),
        };

        #[cfg(unix)]
        handler.register_unix_handlers()?;

        Ok(handler)
    }

    /// Register Unix signal handlers.
    #[cfg(unix)]
    fn register_unix_handlers(&self) -> std::io::Result<()> {
        use std::os::raw::c_int;

        // We use a simple approach: set atomic flags from signal handlers.
        // Signal handlers must be async-signal-safe, so we only use atomics.

        // Store a reference to our state in thread-local storage for the handler.
        // This is a common pattern for signal handlers that need access to state.

        // Note: In production, consider using signal-hook or tokio's signal handling
        // for more robust signal management. This is a simplified implementation.

        static SHUTDOWN_FLAG: AtomicBool = AtomicBool::new(false);
        static RELOAD_FLAG: AtomicBool = AtomicBool::new(false);

        // Copy our state pointers to statics that the handler can access
        // This is safe because we're setting up the handler before any signals arrive
        let state = Arc::clone(&self.state);

        // Spawn a thread to poll the static flags and update our state
        std::thread::spawn(move || {
            loop {
                if SHUTDOWN_FLAG.swap(false, Ordering::Relaxed) {
                    info!("Shutdown signal received");
                    state.request_shutdown();
                    state.record_signal(SignalKind::Terminate);
                }
                if RELOAD_FLAG.swap(false, Ordering::Relaxed) {
                    info!("Reload signal received");
                    state.request_reload();
                    state.record_signal(SignalKind::Hangup);
                }
                if state.shutdown_requested() {
                    // Exit the poll thread when shutdown is complete
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        });

        // Set up actual signal handlers using libc
        unsafe {
            // SIGTERM handler
            libc::signal(libc::SIGTERM, sigterm_handler as libc::sighandler_t);
            // SIGINT handler
            libc::signal(libc::SIGINT, sigint_handler as libc::sighandler_t);
            // SIGHUP handler
            libc::signal(libc::SIGHUP, sighup_handler as libc::sighandler_t);
        }

        extern "C" fn sigterm_handler(_: c_int) {
            SHUTDOWN_FLAG.store(true, Ordering::Relaxed);
        }

        extern "C" fn sigint_handler(_: c_int) {
            SHUTDOWN_FLAG.store(true, Ordering::Relaxed);
        }

        extern "C" fn sighup_handler(_: c_int) {
            RELOAD_FLAG.store(true, Ordering::Relaxed);
        }

        debug!("Unix signal handlers registered");
        Ok(())
    }

    /// Check if shutdown has been requested.
    #[inline]
    pub fn shutdown_requested(&self) -> bool {
        self.state.shutdown_requested()
    }

    /// Check if reload has been requested (clears the flag).
    #[inline]
    pub fn take_reload_request(&self) -> bool {
        self.state.take_reload_request()
    }

    /// Manually request shutdown.
    pub fn request_shutdown(&self) {
        info!("Manual shutdown requested");
        self.state.request_shutdown();
    }

    /// Get the signal state for inspection.
    pub fn state(&self) -> &SignalState {
        &self.state
    }
}

impl Default for SignalHandler {
    fn default() -> Self {
        Self::new().expect("Failed to create signal handler")
    }
}

/// Block until a shutdown signal is received or timeout expires.
///
/// This is useful for simple applications that just need to wait for shutdown.
///
/// # Returns
///
/// `true` if shutdown was signaled, `false` if timeout expired.
pub fn wait_for_shutdown(handler: &SignalHandler, timeout: std::time::Duration) -> bool {
    let start = std::time::Instant::now();
    let poll_interval = std::time::Duration::from_millis(50);

    while start.elapsed() < timeout {
        if handler.shutdown_requested() {
            return true;
        }
        std::thread::sleep(poll_interval.min(timeout - start.elapsed()));
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signal_state_default() {
        let state = SignalState::new();
        assert!(!state.shutdown_requested());
        assert!(!state.take_reload_request());
        assert_eq!(state.signal_count(), 0);
    }

    #[test]
    fn test_shutdown_request() {
        let state = SignalState::new();
        assert!(!state.shutdown_requested());

        state.request_shutdown();
        assert!(state.shutdown_requested());
    }

    #[test]
    fn test_reload_request() {
        let state = SignalState::new();
        assert!(!state.take_reload_request());

        state.request_reload();
        assert!(state.take_reload_request());
        // Flag should be cleared after take
        assert!(!state.take_reload_request());
    }

    #[test]
    fn test_signal_handler_manual_shutdown() {
        let handler = SignalHandler::new().unwrap();
        assert!(!handler.shutdown_requested());

        handler.request_shutdown();
        assert!(handler.shutdown_requested());
    }
}
