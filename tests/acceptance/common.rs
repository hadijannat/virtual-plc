//! Common utilities for integration tests.
//!
//! Provides helpers for:
//! - Checking real-time prerequisites (PREEMPT_RT, privileges)
//! - Collecting timing metrics
//! - Generating test reports

#![allow(dead_code)] // Some utilities are for future soak tests

use std::fs;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Global counter for unique temp file names.
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Result of a latency measurement session.
#[derive(Debug, Clone, Default)]
pub struct LatencyStats {
    /// Minimum latency in microseconds.
    pub min_us: u64,
    /// Average latency in microseconds.
    pub avg_us: u64,
    /// Maximum latency in microseconds.
    pub max_us: u64,
    /// 99th percentile in microseconds.
    pub p99_us: u64,
    /// 99.9th percentile in microseconds.
    pub p999_us: u64,
    /// 99.999th percentile in microseconds.
    pub p99999_us: u64,
    /// Total number of samples.
    pub samples: u64,
    /// Number of overruns (missed deadlines).
    pub overruns: u64,
}

/// Result of a soak test session.
#[derive(Debug, Clone)]
pub struct SoakResult {
    /// Test duration.
    pub duration: Duration,
    /// Total cycles executed.
    pub total_cycles: u64,
    /// Number of faults detected.
    pub faults: u64,
    /// Peak memory usage in bytes.
    pub peak_memory_bytes: u64,
    /// Latency statistics.
    pub latency: LatencyStats,
    /// Whether the test passed acceptance criteria.
    pub passed: bool,
}

/// Check if the system has PREEMPT_RT kernel.
pub fn has_preempt_rt() -> bool {
    if let Ok(version) = fs::read_to_string("/proc/version") {
        version.contains("PREEMPT_RT") || version.contains("PREEMPT RT")
    } else {
        false
    }
}

/// Check if running as root (required for RT priority).
pub fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

/// Check if cyclictest is available.
pub fn has_cyclictest() -> bool {
    Command::new("which")
        .arg("cyclictest")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Check if stress-ng is available.
pub fn has_stress_ng() -> bool {
    Command::new("which")
        .arg("stress-ng")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Get the number of CPUs.
pub fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(1)
}

/// Check all prerequisites for real-time tests.
pub fn check_rt_prerequisites() -> Result<(), String> {
    let mut warnings = Vec::new();
    let mut errors = Vec::new();

    if !is_root() {
        errors.push("Not running as root - RT priority tests will fail");
    }

    if !has_preempt_rt() {
        warnings.push("PREEMPT_RT kernel not detected - latency results may be unreliable");
    }

    if !has_cyclictest() {
        errors.push("cyclictest not found - install rt-tests package");
    }

    if !has_stress_ng() {
        warnings.push("stress-ng not found - load generation will be limited");
    }

    for warning in &warnings {
        eprintln!("WARNING: {}", warning);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// Generate a unique temp file path for cyclictest histogram.
fn unique_histfile_path() -> String {
    let pid = std::process::id();
    let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("/tmp/cyclictest_hist_{}_{}.txt", pid, counter)
}

/// Run cyclictest and parse results.
///
/// # Arguments
///
/// * `duration_secs` - Test duration in seconds
/// * `interval_us` - Cycle interval in microseconds
/// * `priority` - RT priority (1-99)
/// * `cpu` - CPU to pin to (None for all CPUs)
pub fn run_cyclictest(
    duration_secs: u64,
    interval_us: u64,
    priority: u32,
    cpu: Option<usize>,
) -> Result<LatencyStats, String> {
    // Generate unique histogram file path
    let histfile = unique_histfile_path();

    let mut cmd = Command::new("cyclictest");

    cmd.arg("--mlockall")
        .arg("--priority")
        .arg(priority.to_string())
        .arg("--interval")
        .arg(interval_us.to_string())
        .arg("--duration")
        .arg(duration_secs.to_string())
        .arg("--histogram=1000")
        .arg(format!("--histfile={}", histfile));

    if let Some(c) = cpu {
        cmd.arg("--affinity").arg(c.to_string());
    } else {
        cmd.arg("--smp");
    }

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to run cyclictest: {}", e))?;

    // Clean up histogram file after reading
    let result = if !output.status.success() {
        Err(format!(
            "cyclictest failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    } else {
        parse_cyclictest_output(&String::from_utf8_lossy(&output.stdout), &histfile)
    };

    // Always try to clean up the temp file
    let _ = fs::remove_file(&histfile);

    result
}

/// Parse cyclictest output to extract statistics.
fn parse_cyclictest_output(output: &str, histfile: &str) -> Result<LatencyStats, String> {
    let mut stats = LatencyStats::default();

    for line in output.lines() {
        // Parse lines like: "T: 0 ( 1234) P:99 I:1000 C:  10000 Min:      1 Act:    5 Avg:    3 Max:      42"
        if line.starts_with("T:") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            for (i, part) in parts.iter().enumerate() {
                // Handle both "Min: 1" (separate tokens) and "Min:1" (joined)
                if *part == "Min:" && i + 1 < parts.len() {
                    stats.min_us = parts[i + 1].parse().unwrap_or(0);
                } else if let Some(val) = part.strip_prefix("Min:") {
                    if let Ok(n) = val.parse::<u64>() {
                        stats.min_us = n;
                    }
                } else if *part == "Avg:" && i + 1 < parts.len() {
                    stats.avg_us = parts[i + 1].parse().unwrap_or(0);
                } else if let Some(val) = part.strip_prefix("Avg:") {
                    if let Ok(n) = val.parse::<u64>() {
                        stats.avg_us = n;
                    }
                } else if *part == "Max:" && i + 1 < parts.len() {
                    stats.max_us = parts[i + 1].parse().unwrap_or(0);
                } else if let Some(val) = part.strip_prefix("Max:") {
                    if let Ok(n) = val.parse::<u64>() {
                        stats.max_us = n;
                    }
                }
            }

            // Parse C: (cycles) - it appears as "C:" followed by a number
            // Format: "C:  10000" or "C:10000"
            if let Some(c_pos) = line.find("C:") {
                let after_c = &line[c_pos + 2..];
                let num_str: String = after_c
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || c.is_whitespace())
                    .collect();
                if let Ok(n) = num_str.trim().parse::<u64>() {
                    stats.samples = stats.samples.saturating_add(n);
                }
            }
        }

        // Parse overrun count from lines like: "Total: 000001234"
        // cyclictest reports overruns with "OVERRUN" in the output or in summary
        if line.contains("overrun") || line.contains("OVERRUN") {
            // Try to extract number from the line
            for word in line.split_whitespace() {
                if let Ok(n) = word.parse::<u64>() {
                    stats.overruns = stats.overruns.saturating_add(n);
                }
            }
        }
    }

    // Try to parse histogram for percentiles
    match fs::read_to_string(histfile) {
        Ok(hist_content) => {
            parse_histogram_percentiles(&hist_content, &mut stats);
        }
        Err(e) => {
            // Histogram file is required for accurate percentile data
            return Err(format!(
                "Failed to read histogram file {}: {}. Stats may be incomplete.",
                histfile, e
            ));
        }
    }

    // Validate that we got meaningful data
    if stats.samples == 0 {
        return Err("No samples collected from cyclictest".to_string());
    }

    Ok(stats)
}

/// Parse histogram file for percentile calculations.
fn parse_histogram_percentiles(content: &str, stats: &mut LatencyStats) {
    let mut histogram: Vec<(u64, u64)> = Vec::new();
    let mut total: u64 = 0;

    for line in content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            if let (Ok(latency), Ok(count)) = (parts[0].parse::<u64>(), parts[1].parse::<u64>()) {
                histogram.push((latency, count));
                total += count;
            }
        }
    }

    if total == 0 {
        return;
    }

    stats.samples = total;

    // Calculate percentiles
    let p99_threshold = (total as f64 * 0.99) as u64;
    let p999_threshold = (total as f64 * 0.999) as u64;
    let p99999_threshold = (total as f64 * 0.99999) as u64;

    let mut cumulative: u64 = 0;
    for (latency, count) in histogram {
        cumulative += count;
        if stats.p99_us == 0 && cumulative >= p99_threshold {
            stats.p99_us = latency;
        }
        if stats.p999_us == 0 && cumulative >= p999_threshold {
            stats.p999_us = latency;
        }
        if stats.p99999_us == 0 && cumulative >= p99999_threshold {
            stats.p99999_us = latency;
        }
    }
}

/// Start stress-ng load generators.
///
/// Returns the child process handle.
pub fn start_stress_ng(
    cpu_workers: usize,
    io_workers: usize,
) -> Result<std::process::Child, String> {
    Command::new("stress-ng")
        .arg("--cpu")
        .arg(cpu_workers.to_string())
        .arg("--io")
        .arg(io_workers.to_string())
        .arg("--vm")
        .arg("1")
        .arg("--vm-bytes")
        .arg("256M")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to start stress-ng: {}", e))
}

/// Get current process memory usage in bytes.
pub fn get_memory_usage() -> u64 {
    if let Ok(status) = fs::read_to_string("/proc/self/status") {
        for line in status.lines() {
            if line.starts_with("VmRSS:") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Ok(kb) = parts[1].parse::<u64>() {
                        return kb * 1024;
                    }
                }
            }
        }
    }
    0
}

/// Generate a JSON report from test results.
pub fn generate_report(
    test_name: &str,
    latency: &LatencyStats,
    passed: bool,
    notes: &str,
) -> String {
    format!(
        r#"{{
  "test": "{}",
  "passed": {},
  "latency": {{
    "min_us": {},
    "avg_us": {},
    "max_us": {},
    "p99_us": {},
    "p999_us": {},
    "p99999_us": {},
    "samples": {},
    "overruns": {}
  }},
  "notes": "{}"
}}"#,
        test_name,
        passed,
        latency.min_us,
        latency.avg_us,
        latency.max_us,
        latency.p99_us,
        latency.p999_us,
        latency.p99999_us,
        latency.samples,
        latency.overruns,
        notes
    )
}

/// Acceptance criteria for latency tests.
pub struct AcceptanceCriteria {
    /// Maximum acceptable 99.999th percentile latency in microseconds.
    pub max_p99999_us: u64,
    /// Maximum acceptable worst-case latency in microseconds.
    pub max_latency_us: u64,
    /// Maximum acceptable overrun count.
    pub max_overruns: u64,
}

impl Default for AcceptanceCriteria {
    fn default() -> Self {
        Self {
            max_p99999_us: 50, // 50µs per plan
            max_latency_us: 100,
            max_overruns: 0,
        }
    }
}

impl AcceptanceCriteria {
    /// Check if latency stats meet acceptance criteria.
    pub fn check(&self, stats: &LatencyStats) -> bool {
        stats.p99999_us <= self.max_p99999_us
            && stats.max_us <= self.max_latency_us
            && stats.overruns <= self.max_overruns
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_acceptance_criteria_default() {
        let criteria = AcceptanceCriteria::default();
        assert_eq!(criteria.max_p99999_us, 50);
    }

    #[test]
    fn test_parse_cyclictest_c_token() {
        // Create a temporary histogram file with sample data
        let mut histfile = NamedTempFile::new().unwrap();
        writeln!(histfile, "1 5000").unwrap();
        writeln!(histfile, "2 3000").unwrap();
        writeln!(histfile, "3 2000").unwrap();
        let histpath = histfile.path().to_str().unwrap();

        // Test standard cyclictest output format
        let output =
            "T: 0 ( 1234) P:99 I:1000 C:  10000 Min:      1 Act:    5 Avg:    3 Max:      42";
        let result = parse_cyclictest_output(output, histpath);
        assert!(result.is_ok(), "Parse should succeed: {:?}", result);
        let stats = result.unwrap();
        assert_eq!(stats.min_us, 1);
        assert_eq!(stats.avg_us, 3);
        assert_eq!(stats.max_us, 42);
        // samples comes from histogram (5000+3000+2000=10000)
        assert_eq!(stats.samples, 10000);
    }

    #[test]
    fn test_parse_cyclictest_c_token_no_spaces() {
        let mut histfile = NamedTempFile::new().unwrap();
        writeln!(histfile, "1 5000").unwrap();
        let histpath = histfile.path().to_str().unwrap();

        // Test C: without spaces
        let output = "T: 0 ( 1234) P:99 I:1000 C:5000 Min:1 Act:5 Avg:3 Max:42";
        let result = parse_cyclictest_output(output, histpath);
        assert!(result.is_ok());
        let stats = result.unwrap();
        assert_eq!(stats.min_us, 1);
        assert_eq!(stats.max_us, 42);
    }

    #[test]
    fn test_parse_cyclictest_multiple_threads() {
        let mut histfile = NamedTempFile::new().unwrap();
        writeln!(histfile, "1 15000").unwrap();
        let histpath = histfile.path().to_str().unwrap();

        // Multiple T: lines (multi-CPU)
        let output = "\
T: 0 ( 1234) P:99 I:1000 C:  5000 Min:      1 Act:    5 Avg:    3 Max:      20
T: 1 ( 1235) P:99 I:1000 C:  5000 Min:      2 Act:    6 Avg:    4 Max:      25
T: 2 ( 1236) P:99 I:1000 C:  5000 Min:      1 Act:    4 Avg:    3 Max:      30";

        let result = parse_cyclictest_output(output, histpath);
        assert!(result.is_ok());
        let stats = result.unwrap();
        // Last T: line values are used for min/avg/max (could be improved to aggregate)
        assert_eq!(stats.max_us, 30);
    }

    #[test]
    fn test_parse_cyclictest_overruns() {
        let mut histfile = NamedTempFile::new().unwrap();
        writeln!(histfile, "1 1000").unwrap();
        let histpath = histfile.path().to_str().unwrap();

        // Test overrun detection
        let output = "\
T: 0 ( 1234) P:99 I:1000 C:  1000 Min:      1 Act:    5 Avg:    3 Max:      42
WARN: 5 overrun detected";

        let result = parse_cyclictest_output(output, histpath);
        assert!(result.is_ok());
        let stats = result.unwrap();
        assert_eq!(stats.overruns, 5);
    }

    #[test]
    fn test_acceptance_criteria_pass() {
        let criteria = AcceptanceCriteria::default();
        let stats = LatencyStats {
            min_us: 1,
            avg_us: 5,
            max_us: 45,
            p99_us: 20,
            p999_us: 30,
            p99999_us: 40,
            samples: 10000,
            overruns: 0,
        };
        assert!(criteria.check(&stats));
    }

    #[test]
    fn test_acceptance_criteria_fail_p99999() {
        let criteria = AcceptanceCriteria::default();
        let stats = LatencyStats {
            p99999_us: 60, // Exceeds 50µs
            max_us: 60,
            ..Default::default()
        };
        assert!(!criteria.check(&stats));
    }

    #[test]
    fn test_generate_report() {
        let stats = LatencyStats {
            min_us: 1,
            avg_us: 5,
            max_us: 20,
            ..Default::default()
        };
        let report = generate_report("test", &stats, true, "ok");
        assert!(report.contains("\"test\": \"test\""));
        assert!(report.contains("\"passed\": true"));
    }
}
