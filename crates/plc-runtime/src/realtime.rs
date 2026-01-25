//! Real-time scheduling and memory locking utilities.
//!
//! Provides platform-specific initialization for deterministic execution:
//! - Memory locking (mlockall) to prevent page faults
//! - Stack pre-faulting to ensure stack pages are resident
//! - Real-time scheduling (SCHED_FIFO/SCHED_RR) for priority execution
//! - CPU affinity to isolate RT threads from system housekeeping

#![allow(unused_imports)] // Platform-specific code may not use all imports

use plc_common::config::{CpuAffinity, RealtimeConfig, SchedPolicy};
use plc_common::error::{PlcError, PlcResult};
use tracing::{debug, error, info, warn};

/// Result of real-time initialization.
#[derive(Debug, Clone)]
pub struct RealtimeStatus {
    /// Whether memory was locked successfully.
    pub memory_locked: bool,
    /// Stack bytes pre-faulted.
    pub stack_prefaulted: usize,
    /// Applied scheduler policy.
    pub scheduler_policy: Option<SchedPolicy>,
    /// Applied scheduler priority.
    pub scheduler_priority: Option<u8>,
    /// CPUs the thread is pinned to.
    pub cpu_affinity: Option<Vec<usize>>,
}

/// Initialize real-time environment based on configuration.
///
/// # Errors
///
/// Returns an error if a required RT feature fails to initialize.
/// Non-fatal warnings are logged but don't cause failure.
///
/// # Platform Support
///
/// Full support on Linux with PREEMPT_RT kernel.
/// Partial/no-op on macOS and other platforms.
pub fn init_realtime(config: &RealtimeConfig) -> PlcResult<RealtimeStatus> {
    if !config.enabled {
        info!("Real-time scheduling disabled in configuration");
        return Ok(RealtimeStatus {
            memory_locked: false,
            stack_prefaulted: 0,
            scheduler_policy: None,
            scheduler_priority: None,
            cpu_affinity: None,
        });
    }

    // If fail_fast is enabled, validate RT capabilities before proceeding
    if config.fail_fast {
        info!("Validating real-time capabilities (fail_fast=true)");
        validate_rt_capabilities(config)?;
    }

    info!("Initializing real-time environment");

    let memory_locked = if config.lock_memory {
        lock_memory()?
    } else {
        false
    };

    let stack_prefaulted = prefault_stack(config.prefault_stack_size);

    let (scheduler_policy, scheduler_priority) = set_scheduler(config.policy, config.priority)?;

    let cpu_affinity = set_cpu_affinity(&config.cpu_affinity)?;

    let status = RealtimeStatus {
        memory_locked,
        stack_prefaulted,
        scheduler_policy,
        scheduler_priority,
        cpu_affinity,
    };

    info!(?status, "Real-time initialization complete");
    Ok(status)
}

/// Lock all current and future memory pages.
#[cfg(target_os = "linux")]
fn lock_memory() -> PlcResult<bool> {
    use nix::sys::mman::{mlockall, MlockAllFlags};

    debug!("Locking memory pages with mlockall");

    match mlockall(MlockAllFlags::MCL_CURRENT | MlockAllFlags::MCL_FUTURE) {
        Ok(()) => {
            info!("Memory locked successfully");
            Ok(true)
        }
        Err(e) => {
            // EPERM is common when not running as root or without CAP_IPC_LOCK
            if e == nix::errno::Errno::EPERM {
                warn!(
                    "mlockall failed with EPERM - running without CAP_IPC_LOCK capability. \
                     Page faults may occur during execution."
                );
                Ok(false)
            } else {
                Err(PlcError::Config(format!("mlockall failed: {e}")))
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn lock_memory() -> PlcResult<bool> {
    warn!("mlockall not available on this platform");
    Ok(false)
}

/// Pre-fault stack pages to avoid page faults during execution.
///
/// This function allocates a buffer on the stack and touches each page
/// to ensure they are resident in memory before the RT loop begins.
fn prefault_stack(size: usize) -> usize {
    if size == 0 {
        return 0;
    }

    debug!(size, "Pre-faulting stack pages");

    // Use a volatile write to prevent the compiler from optimizing this away
    // We allocate in chunks to avoid stack overflow on systems with small default stacks
    const CHUNK_SIZE: usize = 64 * 1024; // 64 KiB chunks
    let mut total_faulted = 0;

    // We can't actually allocate `size` bytes on the stack in one go,
    // so we do it iteratively with a recursive helper or loop.
    // For safety, we'll use a heap allocation that simulates stack behavior.

    let pages_to_fault = size / page_size();
    let page_sz = page_size();

    for _ in 0..pages_to_fault.min(CHUNK_SIZE / page_sz) {
        // Touch each page with a volatile write
        let mut page = vec![0u8; page_sz];
        // SAFETY: We're writing to our own allocation
        unsafe {
            std::ptr::write_volatile(page.as_mut_ptr(), 0xAA);
        }
        total_faulted += page_sz;
        // Drop page - this is just to fault pages, not to keep them
    }

    // For actual stack pre-faulting, use a recursive approach
    total_faulted += prefault_stack_recursive(size.saturating_sub(total_faulted), 0);

    debug!(total_faulted, "Stack pre-fault complete");
    total_faulted
}

/// Recursive helper to actually fault stack pages.
#[inline(never)]
fn prefault_stack_recursive(remaining: usize, depth: usize) -> usize {
    const FRAME_SIZE: usize = 4096; // Approximate stack frame size
    const MAX_DEPTH: usize = 1000; // Limit recursion depth

    if remaining < FRAME_SIZE || depth >= MAX_DEPTH {
        return 0;
    }

    // Allocate a buffer on the actual stack
    let mut buffer = [0u8; FRAME_SIZE];

    // Touch the buffer with volatile writes to prevent optimization
    // SAFETY: Writing to our own stack allocation
    unsafe {
        std::ptr::write_volatile(buffer.as_mut_ptr(), 0xBB);
        std::ptr::write_volatile(buffer.as_mut_ptr().add(FRAME_SIZE - 1), 0xCC);
    }

    // Prevent the buffer from being optimized away
    std::hint::black_box(&buffer);

    FRAME_SIZE + prefault_stack_recursive(remaining - FRAME_SIZE, depth + 1)
}

/// Get system page size.
fn page_size() -> usize {
    // SAFETY: sysconf is safe to call
    #[cfg(unix)]
    {
        unsafe { libc::sysconf(libc::_SC_PAGESIZE) as usize }
    }
    #[cfg(not(unix))]
    {
        4096 // Default assumption
    }
}

/// Set real-time scheduler policy and priority.
#[cfg(target_os = "linux")]
fn set_scheduler(
    policy: SchedPolicy,
    priority: u8,
) -> PlcResult<(Option<SchedPolicy>, Option<u8>)> {
    use nix::sched::{sched_setaffinity, CpuSet};
    use nix::unistd::Pid;

    let linux_policy = match policy {
        SchedPolicy::Fifo => libc::SCHED_FIFO,
        SchedPolicy::Rr => libc::SCHED_RR,
        SchedPolicy::Other => {
            debug!("Using SCHED_OTHER (non-RT) scheduling");
            return Ok((Some(SchedPolicy::Other), None));
        }
    };

    // Clamp priority to valid range (1-99 for RT policies)
    let clamped_priority = priority.clamp(1, 99);
    if clamped_priority != priority {
        warn!(
            original = priority,
            clamped = clamped_priority,
            "Scheduler priority clamped to valid range"
        );
    }

    debug!(
        ?policy,
        priority = clamped_priority,
        "Setting real-time scheduler"
    );

    // SAFETY: sched_setscheduler is safe when called with valid parameters
    let param = libc::sched_param {
        sched_priority: i32::from(clamped_priority),
    };

    let result = unsafe { libc::sched_setscheduler(0, linux_policy, &param) };

    if result == -1 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::EPERM) {
            warn!(
                "sched_setscheduler failed with EPERM - running without RT privileges. \
                 Consider running with CAP_SYS_NICE capability or as root."
            );
            return Ok((None, None));
        }
        return Err(PlcError::Config(format!(
            "sched_setscheduler failed: {err}"
        )));
    }

    info!(
        ?policy,
        priority = clamped_priority,
        "Real-time scheduler configured"
    );
    Ok((Some(policy), Some(clamped_priority)))
}

#[cfg(not(target_os = "linux"))]
fn set_scheduler(
    policy: SchedPolicy,
    priority: u8,
) -> PlcResult<(Option<SchedPolicy>, Option<u8>)> {
    warn!(
        ?policy,
        priority, "Real-time scheduling not available on this platform"
    );
    Ok((None, None))
}

/// Set CPU affinity for the current thread.
#[cfg(target_os = "linux")]
fn set_cpu_affinity(affinity: &CpuAffinity) -> PlcResult<Option<Vec<usize>>> {
    use nix::sched::{sched_setaffinity, CpuSet};
    use nix::unistd::Pid;

    let cpus = match affinity {
        CpuAffinity::None => {
            debug!("No CPU affinity configured");
            return Ok(None);
        }
        CpuAffinity::Single(cpu) => vec![*cpu],
        CpuAffinity::Set(cpus) => cpus.clone(),
    };

    if cpus.is_empty() {
        return Ok(None);
    }

    debug!(?cpus, "Setting CPU affinity");

    let mut cpu_set = CpuSet::new();
    for &cpu in &cpus {
        cpu_set
            .set(cpu)
            .map_err(|e| PlcError::Config(format!("Invalid CPU index {cpu}: {e}")))?;
    }

    match sched_setaffinity(Pid::from_raw(0), &cpu_set) {
        Ok(()) => {
            info!(?cpus, "CPU affinity set");
            Ok(Some(cpus))
        }
        Err(e) => {
            if e == nix::errno::Errno::EINVAL {
                warn!(?cpus, "Invalid CPU set - some CPUs may not exist");
                Ok(None)
            } else {
                Err(PlcError::Config(format!("sched_setaffinity failed: {e}")))
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn set_cpu_affinity(affinity: &CpuAffinity) -> PlcResult<Option<Vec<usize>>> {
    if !matches!(affinity, CpuAffinity::None) {
        warn!("CPU affinity not available on this platform");
    }
    Ok(None)
}

/// Check if the current process has real-time capabilities.
#[cfg(target_os = "linux")]
pub fn check_rt_capabilities() -> RtCapabilities {
    use std::fs;

    let mut caps = RtCapabilities {
        is_root: unsafe { libc::geteuid() } == 0,
        ..Default::default()
    };

    // Check RLIMIT_RTPRIO
    let mut rlim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { libc::getrlimit(libc::RLIMIT_RTPRIO, &mut rlim) } == 0 {
        caps.rtprio_limit = Some(rlim.rlim_cur);
    }

    // Check RLIMIT_MEMLOCK
    if unsafe { libc::getrlimit(libc::RLIMIT_MEMLOCK, &mut rlim) } == 0 {
        caps.memlock_limit = Some(rlim.rlim_cur);
    }

    // Check for PREEMPT_RT kernel
    if let Ok(version) = fs::read_to_string("/proc/version") {
        caps.preempt_rt = version.contains("PREEMPT_RT") || version.contains("PREEMPT RT");
    }

    caps
}

#[cfg(not(target_os = "linux"))]
pub fn check_rt_capabilities() -> RtCapabilities {
    RtCapabilities::default()
}

/// Information about real-time capabilities of the system.
#[derive(Debug, Clone, Default)]
pub struct RtCapabilities {
    /// Whether running as root.
    pub is_root: bool,
    /// RLIMIT_RTPRIO value (max RT priority allowed).
    pub rtprio_limit: Option<u64>,
    /// RLIMIT_MEMLOCK value (max lockable memory).
    pub memlock_limit: Option<u64>,
    /// Whether running on a PREEMPT_RT kernel.
    pub preempt_rt: bool,
}

impl RtCapabilities {
    /// Check if RT scheduling is likely to succeed.
    pub fn can_use_rt_scheduling(&self) -> bool {
        self.is_root || self.rtprio_limit.is_some_and(|l| l > 0)
    }

    /// Check if memory locking is likely to succeed.
    pub fn can_lock_memory(&self) -> bool {
        if self.is_root {
            return true;
        }

        #[cfg(target_family = "unix")]
        {
            self.memlock_limit.is_some_and(|l| l == libc::RLIM_INFINITY)
        }

        #[cfg(not(target_family = "unix"))]
        {
            false
        }
    }
}

/// Validate that real-time capabilities are available.
///
/// This function is called when `fail_fast` is enabled in the realtime config.
/// It checks for required RT capabilities and returns an error if any are missing.
///
/// # Errors
///
/// Returns an error describing which RT requirements are not met:
/// - PREEMPT_RT kernel not detected
/// - CAP_SYS_NICE / RLIMIT_RTPRIO not available
/// - CAP_IPC_LOCK / RLIMIT_MEMLOCK not available
pub fn validate_rt_capabilities(config: &RealtimeConfig) -> PlcResult<()> {
    if !config.enabled {
        // RT not enabled, nothing to validate
        return Ok(());
    }

    let caps = check_rt_capabilities();
    let mut issues = Vec::new();

    // Check for PREEMPT_RT kernel (warning, not fatal)
    if !caps.preempt_rt {
        warn!(
            "PREEMPT_RT kernel not detected. Real-time performance may be degraded. \
             For production deployments, use a kernel with PREEMPT_RT patches."
        );
        // Note: We warn but don't fail on this, as vanilla kernels can still work
        // for soft real-time. Add to issues only for informational purposes.
    }

    // Check for RT scheduling capability
    if config.policy != SchedPolicy::Other && !caps.can_use_rt_scheduling() {
        issues.push(format!(
            "Cannot use RT scheduling (SCHED_{:?}): RLIMIT_RTPRIO={:?}, is_root={}. \
             Grant CAP_SYS_NICE capability or set RLIMIT_RTPRIO > 0.",
            config.policy, caps.rtprio_limit, caps.is_root
        ));
    }

    // Check for memory locking capability
    if config.lock_memory && !caps.can_lock_memory() {
        issues.push(format!(
            "Cannot lock memory: RLIMIT_MEMLOCK={:?}, is_root={}. \
             Grant CAP_IPC_LOCK capability or set RLIMIT_MEMLOCK to unlimited.",
            caps.memlock_limit, caps.is_root
        ));
    }

    if issues.is_empty() {
        info!("Real-time capabilities validated successfully");
        Ok(())
    } else {
        let message = format!(
            "Real-time requirements not met (fail_fast=true):\n  - {}",
            issues.join("\n  - ")
        );
        error!("{}", message);
        Err(PlcError::Config(message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disabled_rt() {
        let config = RealtimeConfig {
            enabled: false,
            ..Default::default()
        };

        let status = init_realtime(&config).unwrap();
        assert!(!status.memory_locked);
        assert_eq!(status.stack_prefaulted, 0);
        assert!(status.scheduler_policy.is_none());
    }

    #[test]
    fn test_page_size() {
        let ps = page_size();
        assert!(ps > 0);
        assert!(ps.is_power_of_two());
    }

    #[test]
    fn test_stack_prefault() {
        let faulted = prefault_stack(64 * 1024); // 64 KiB
        assert!(faulted > 0);
    }

    #[test]
    fn test_rt_capabilities() {
        let caps = check_rt_capabilities();
        // Just verify it doesn't panic
        let _ = caps.can_use_rt_scheduling();
        let _ = caps.can_lock_memory();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_cpu_affinity_none() {
        let result = set_cpu_affinity(&CpuAffinity::None).unwrap();
        assert!(result.is_none());
    }
}
