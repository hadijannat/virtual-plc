//! Latency and jitter acceptance tests.
//!
//! These tests verify that the system meets real-time latency requirements
//! using cyclictest as the measurement tool.
//!
//! # Requirements
//!
//! - Root privileges
//! - PREEMPT_RT kernel (highly recommended)
//! - cyclictest from rt-tests package
//! - stress-ng for load generation
//!
//! # Acceptance Criteria
//!
//! - 99.999th percentile jitter < 50µs under load
//! - Maximum latency < 100µs
//! - Zero deadline overruns

use super::common::{
    check_rt_prerequisites, has_stress_ng, num_cpus, run_cyclictest,
    start_stress_ng, AcceptanceCriteria,
};
use std::time::Duration;

/// Test latency without any system load.
/// This establishes the baseline performance.
#[test]
#[ignore = "Requires root and cyclictest"]
fn test_latency_no_load() {
    if let Err(e) = check_rt_prerequisites() {
        eprintln!("Skipping test: {}", e);
        return;
    }

    println!("Running latency test without load...");

    let stats = run_cyclictest(
        10, // 10 seconds
        1000, // 1ms interval
        99,   // Max RT priority
        None, // All CPUs
    )
    .expect("cyclictest failed");

    println!("Results (no load):");
    println!("  Min: {} µs", stats.min_us);
    println!("  Avg: {} µs", stats.avg_us);
    println!("  Max: {} µs", stats.max_us);
    println!("  Samples: {}", stats.samples);

    let criteria = AcceptanceCriteria::default();
    assert!(
        criteria.check(&stats),
        "Latency test failed: max={}µs, p99999={}µs",
        stats.max_us,
        stats.p99999_us
    );
}

/// Test latency under CPU load.
/// Verifies jitter stays bounded when system is busy.
#[test]
#[ignore = "Requires root, cyclictest, and stress-ng"]
fn test_latency_under_cpu_load() {
    if let Err(e) = check_rt_prerequisites() {
        eprintln!("Skipping test: {}", e);
        return;
    }

    if !has_stress_ng() {
        eprintln!("Skipping test: stress-ng not available");
        return;
    }

    let cpus = num_cpus();
    println!(
        "Running latency test under CPU load ({} workers)...",
        cpus
    );

    // Start stress-ng
    let mut stress = start_stress_ng(cpus, 0).expect("Failed to start stress-ng");

    // Give stress-ng time to ramp up
    std::thread::sleep(Duration::from_secs(2));

    let stats = run_cyclictest(
        30, // 30 seconds
        1000, // 1ms interval
        99,   // Max RT priority
        None, // All CPUs
    );

    // Kill stress-ng
    let _ = stress.kill();
    let _ = stress.wait();

    let stats = stats.expect("cyclictest failed");

    println!("Results (CPU load):");
    println!("  Min: {} µs", stats.min_us);
    println!("  Avg: {} µs", stats.avg_us);
    println!("  Max: {} µs", stats.max_us);
    println!("  P99: {} µs", stats.p99_us);
    println!("  P99.9: {} µs", stats.p999_us);
    println!("  P99.999: {} µs", stats.p99999_us);
    println!("  Samples: {}", stats.samples);

    let criteria = AcceptanceCriteria::default();
    assert!(
        criteria.check(&stats),
        "Latency test failed under CPU load: max={}µs, p99999={}µs",
        stats.max_us,
        stats.p99999_us
    );
}

/// Test latency under mixed I/O and CPU load.
/// This is the most stressful scenario.
#[test]
#[ignore = "Requires root, cyclictest, and stress-ng - long running"]
fn test_latency_under_mixed_load() {
    if let Err(e) = check_rt_prerequisites() {
        eprintln!("Skipping test: {}", e);
        return;
    }

    if !has_stress_ng() {
        eprintln!("Skipping test: stress-ng not available");
        return;
    }

    let cpus = num_cpus();
    println!(
        "Running latency test under mixed load ({} CPU + {} I/O workers)...",
        cpus, cpus
    );

    // Start stress-ng with CPU, I/O, and memory stress
    let mut stress = start_stress_ng(cpus, cpus).expect("Failed to start stress-ng");

    // Give stress-ng time to ramp up
    std::thread::sleep(Duration::from_secs(3));

    let stats = run_cyclictest(
        60, // 60 seconds
        1000, // 1ms interval
        99,   // Max RT priority
        None, // All CPUs
    );

    // Kill stress-ng
    let _ = stress.kill();
    let _ = stress.wait();

    let stats = stats.expect("cyclictest failed");

    println!("Results (mixed load):");
    println!("  Min: {} µs", stats.min_us);
    println!("  Avg: {} µs", stats.avg_us);
    println!("  Max: {} µs", stats.max_us);
    println!("  P99: {} µs", stats.p99_us);
    println!("  P99.9: {} µs", stats.p999_us);
    println!("  P99.999: {} µs", stats.p99999_us);
    println!("  Samples: {}", stats.samples);
    println!("  Overruns: {}", stats.overruns);

    let criteria = AcceptanceCriteria::default();
    assert!(
        criteria.check(&stats),
        "Latency test failed under mixed load: max={}µs, p99999={}µs, overruns={}",
        stats.max_us,
        stats.p99999_us,
        stats.overruns
    );
}

/// Extended latency test (10 minutes) under load.
/// Used for pre-deployment validation.
#[test]
#[ignore = "Requires root, cyclictest, stress-ng - very long running (10 min)"]
fn test_latency_extended() {
    if let Err(e) = check_rt_prerequisites() {
        eprintln!("Skipping test: {}", e);
        return;
    }

    if !has_stress_ng() {
        eprintln!("Skipping test: stress-ng not available");
        return;
    }

    let cpus = num_cpus();
    println!("Running extended latency test (10 minutes)...");

    // Start stress-ng
    let mut stress = start_stress_ng(cpus, cpus / 2).expect("Failed to start stress-ng");

    std::thread::sleep(Duration::from_secs(3));

    let stats = run_cyclictest(
        600, // 10 minutes
        1000, // 1ms interval
        99,   // Max RT priority
        None, // All CPUs
    );

    let _ = stress.kill();
    let _ = stress.wait();

    let stats = stats.expect("cyclictest failed");

    println!("Extended test results:");
    println!("  Min: {} µs", stats.min_us);
    println!("  Avg: {} µs", stats.avg_us);
    println!("  Max: {} µs", stats.max_us);
    println!("  P99.999: {} µs", stats.p99999_us);
    println!("  Samples: {}", stats.samples);
    println!("  Overruns: {}", stats.overruns);

    let criteria = AcceptanceCriteria::default();
    assert!(
        criteria.check(&stats),
        "Extended latency test failed: max={}µs, p99999={}µs",
        stats.max_us,
        stats.p99999_us
    );
}

/// Verify latency on isolated CPU.
/// Tests CPU isolation effectiveness.
#[test]
#[ignore = "Requires root, cyclictest, and CPU isolation"]
fn test_latency_isolated_cpu() {
    if let Err(e) = check_rt_prerequisites() {
        eprintln!("Skipping test: {}", e);
        return;
    }

    let cpus = num_cpus();
    if cpus < 2 {
        eprintln!("Skipping test: need at least 2 CPUs for isolation test");
        return;
    }

    // Use last CPU (typically isolated in RT setups)
    let isolated_cpu = cpus - 1;
    println!("Running latency test on isolated CPU {}...", isolated_cpu);

    // Start stress on other CPUs
    let mut stress = if has_stress_ng() {
        Some(start_stress_ng(cpus - 1, cpus - 1).expect("Failed to start stress-ng"))
    } else {
        None
    };

    std::thread::sleep(Duration::from_secs(2));

    let stats = run_cyclictest(
        30, // 30 seconds
        1000, // 1ms interval
        99,   // Max RT priority
        Some(isolated_cpu),
    )
    .expect("cyclictest failed");

    if let Some(ref mut s) = stress {
        let _ = s.kill();
        let _ = s.wait();
    }

    println!("Results (isolated CPU {}):", isolated_cpu);
    println!("  Min: {} µs", stats.min_us);
    println!("  Avg: {} µs", stats.avg_us);
    println!("  Max: {} µs", stats.max_us);
    println!("  P99.999: {} µs", stats.p99999_us);

    // Isolated CPU should have much better latency
    let strict_criteria = AcceptanceCriteria {
        max_p99999_us: 20, // Tighter bound for isolated CPU
        max_latency_us: 50,
        max_overruns: 0,
    };

    assert!(
        strict_criteria.check(&stats),
        "Isolated CPU latency test failed: max={}µs, p99999={}µs",
        stats.max_us,
        stats.p99999_us
    );
}
