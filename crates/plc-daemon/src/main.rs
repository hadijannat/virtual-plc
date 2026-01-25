//! PLC daemon entry point.
//!
//! Integrates the scheduler, fieldbus driver, and Wasm logic engine
//! into a complete runtime with signal handling and diagnostics.

mod diagnostics;
mod signals;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use plc_common::config::{FieldbusDriver as FieldbusDriverType, RuntimeConfig};
use plc_common::state::RuntimeState;
use plc_fieldbus::{FieldbusDriver, ModbusTcpConfig, ModbusTcpDriver, SimulatedDriver};
use plc_runtime::scheduler::{Scheduler, SchedulerBuilder};
use plc_runtime::wasm_host::{LogicEngine, NullEngine, WasmtimeHost};
use plc_web_ui::{StateUpdater, WebUiConfig, WebUiServer};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, warn};

use crate::diagnostics::{format_prometheus_metrics, DiagnosticsCollector, DiagnosticsState};
use crate::signals::{wait_for_shutdown, SignalHandler};

/// PLC daemon command-line interface.
#[derive(Parser, Debug)]
#[command(
    name = "plc-daemon",
    about = "Virtual PLC daemon - real-time industrial control runtime",
    version,
    long_about = None
)]
struct Cli {
    /// Log level (trace, debug, info, warn, error).
    #[arg(long, short = 'l', default_value = "info", global = true)]
    log_level: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run the PLC daemon with full runtime.
    Run(RunArgs),

    /// Compile Structured Text to WebAssembly.
    Compile(CompileArgs),

    /// Validate a WebAssembly module for PLC compatibility.
    Validate(ValidateArgs),

    /// Run in simulation mode (simplified, no RT requirements).
    Simulate(SimulateArgs),

    /// Diagnose system real-time capabilities.
    Diagnose(DiagnoseArgs),
}

/// Arguments for the 'run' subcommand.
#[derive(Parser, Debug)]
struct RunArgs {
    /// Path to a runtime configuration file (TOML).
    #[arg(long, short = 'c', value_name = "FILE")]
    config: Option<PathBuf>,

    /// Path to the Wasm logic module (overrides config file).
    #[arg(long, short = 'w', value_name = "FILE")]
    wasm_module: Option<PathBuf>,

    /// Run in simulated fieldbus mode.
    #[arg(long, short = 's')]
    simulated: bool,

    /// Maximum cycles to run (0 = infinite).
    #[arg(long, default_value = "0")]
    max_cycles: u64,
}

/// Arguments for the 'compile' subcommand.
#[derive(Parser, Debug)]
struct CompileArgs {
    /// Input Structured Text file (.st).
    #[arg(value_name = "INPUT")]
    input: PathBuf,

    /// Output WebAssembly file (.wasm).
    #[arg(short = 'o', long, value_name = "OUTPUT")]
    output: Option<PathBuf>,

    /// Emit WAT (WebAssembly Text) instead of binary.
    #[arg(long)]
    wat: bool,

    /// Enable verbose compiler output.
    #[arg(short, long)]
    verbose: bool,
}

/// Arguments for the 'validate' subcommand.
#[derive(Parser, Debug)]
struct ValidateArgs {
    /// WebAssembly module to validate (.wasm or .wat).
    #[arg(value_name = "MODULE")]
    module: PathBuf,

    /// Check for specific exports (comma-separated).
    #[arg(long, default_value = "init,step,memory")]
    exports: String,

    /// Show detailed module information.
    #[arg(short, long)]
    verbose: bool,
}

/// Arguments for the 'simulate' subcommand.
#[derive(Parser, Debug)]
struct SimulateArgs {
    /// WebAssembly module to run (.wasm or .wat).
    #[arg(value_name = "MODULE")]
    module: PathBuf,

    /// Number of cycles to simulate (default: 100).
    #[arg(short = 'n', long, default_value = "100")]
    cycles: u64,

    /// Cycle time in milliseconds (default: 10ms).
    #[arg(long, default_value = "10")]
    cycle_time_ms: u64,

    /// Print I/O state every N cycles (0 = disabled).
    #[arg(long, default_value = "10")]
    print_every: u64,

    /// Set initial digital inputs (hex, e.g., 0xFF).
    #[arg(long, default_value = "0")]
    digital_inputs: String,
}

/// Arguments for the 'diagnose' subcommand.
#[derive(Parser, Debug)]
struct DiagnoseArgs {
    /// Run latency test for specified duration in seconds.
    #[arg(long, default_value = "5")]
    duration: u64,

    /// Target cycle time in microseconds.
    #[arg(long, default_value = "1000")]
    cycle_us: u64,

    /// Output in JSON format for machine parsing.
    #[arg(long)]
    json: bool,

    /// Skip the timing test (system info only).
    #[arg(long)]
    skip_timing_test: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    init_logging(&cli.log_level);

    match cli.command {
        Commands::Run(args) => cmd_run(args),
        Commands::Compile(args) => cmd_compile(args),
        Commands::Validate(args) => cmd_validate(args),
        Commands::Simulate(args) => cmd_simulate(args),
        Commands::Diagnose(args) => cmd_diagnose(args),
    }
}

/// Initialize logging with the specified log level.
fn init_logging(level: &str) {
    let filter = format!(
        "plc_daemon={},plc_runtime={},plc_fieldbus={},plc_common={},plc_compiler={}",
        level, level, level, level, level
    );

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&filter)),
        )
        .with_target(true)
        .with_thread_ids(true)
        .init();
}

// =============================================================================
// SUBCOMMAND: run
// =============================================================================

fn cmd_run(args: RunArgs) -> Result<()> {
    info!(version = env!("CARGO_PKG_VERSION"), "Starting PLC daemon");

    // Load configuration
    let mut config = load_config(&args)?;

    // Override with command-line arguments
    if let Some(wasm_path) = &args.wasm_module {
        config.wasm_module = Some(wasm_path.clone());
    }
    if args.simulated {
        config.fieldbus.driver = FieldbusDriverType::Simulated;
    }

    info!(?config.cycle_time, ?config.fieldbus.driver, "Configuration loaded");

    // Set up signal handling
    let signal_handler = SignalHandler::new().context("Failed to set up signal handlers")?;

    // Set up diagnostics
    let diag_state = Arc::new(DiagnosticsState::new());
    let diagnostics = DiagnosticsCollector::new(Arc::clone(&diag_state));

    // Run the daemon
    run_daemon(&config, &signal_handler, &diagnostics, args.max_cycles)
}

/// Load configuration from file or use defaults.
fn load_config(args: &RunArgs) -> Result<RuntimeConfig> {
    if let Some(config_path) = &args.config {
        info!(?config_path, "Loading config from command-line argument");
        return RuntimeConfig::from_file(config_path)
            .with_context(|| format!("Failed to load config from {:?}", config_path));
    }

    if let Ok(env_path) = std::env::var("PLC_CONFIG_PATH") {
        let config_path = PathBuf::from(&env_path);
        if config_path.exists() {
            info!(?config_path, "Loading config from PLC_CONFIG_PATH");
            return RuntimeConfig::from_file(&config_path).with_context(|| {
                format!("Failed to load config from PLC_CONFIG_PATH={:?}", env_path)
            });
        }
    }

    let system_path = PathBuf::from("/etc/plc/config.toml");
    if system_path.exists() {
        info!(?system_path, "Loading config from system path");
        return RuntimeConfig::from_file(&system_path)
            .with_context(|| format!("Failed to load config from {:?}", system_path));
    }

    let local_path = PathBuf::from("config/default.toml");
    if local_path.exists() {
        info!(?local_path, "Loading config from local path");
        return RuntimeConfig::from_file(&local_path)
            .with_context(|| format!("Failed to load config from {:?}", local_path));
    }

    info!("No config file found, using built-in defaults");
    Ok(RuntimeConfig::default())
}

// =============================================================================
// SUBCOMMAND: compile
// =============================================================================

fn cmd_compile(args: CompileArgs) -> Result<()> {
    info!(input = ?args.input, "Compiling Structured Text");

    // Read source file
    let source = std::fs::read_to_string(&args.input)
        .with_context(|| format!("Failed to read source file: {:?}", args.input))?;

    if args.verbose {
        info!(lines = source.lines().count(), "Source loaded");
    }

    // Compile to Wasm
    let wasm_bytes = plc_compiler::compile(&source)
        .with_context(|| "Compilation failed")?;

    // Determine output path
    let output_path = args.output.unwrap_or_else(|| {
        let mut p = args.input.clone();
        p.set_extension(if args.wat { "wat" } else { "wasm" });
        p
    });

    // Write output
    if args.wat {
        let wat = wasmprinter::print_bytes(&wasm_bytes)
            .with_context(|| "Failed to convert Wasm to WAT")?;
        std::fs::write(&output_path, wat)
            .with_context(|| format!("Failed to write WAT file: {:?}", output_path))?;
    } else {
        std::fs::write(&output_path, &wasm_bytes)
            .with_context(|| format!("Failed to write Wasm file: {:?}", output_path))?;
    }

    println!("Compiled {} -> {} ({} bytes)",
        args.input.display(),
        output_path.display(),
        wasm_bytes.len()
    );

    Ok(())
}

// =============================================================================
// SUBCOMMAND: validate
// =============================================================================

fn cmd_validate(args: ValidateArgs) -> Result<()> {
    info!(module = ?args.module, "Validating WebAssembly module");

    // Read module
    let module_bytes = std::fs::read(&args.module)
        .with_context(|| format!("Failed to read module: {:?}", args.module))?;

    // Check if it's WAT and convert if needed
    let wasm_bytes = if args.module.extension().map_or(false, |e| e == "wat") {
        wat::parse_bytes(&module_bytes)
            .with_context(|| "Failed to parse WAT")?
            .into_owned()
    } else {
        module_bytes
    };

    // Try to create a Wasmtime host and load the module
    let mut host = WasmtimeHost::new(Duration::from_millis(1))
        .with_context(|| "Failed to create Wasm host")?;

    // load_module validates the module and checks for required exports (step, memory)
    host.load_module(&wasm_bytes)
        .with_context(|| "Module failed validation - check for required exports: step, memory")?;

    // Try to initialize - this verifies the module can be instantiated
    // and checks if init export exists
    let init_result = host.init();
    let init_ok = init_result.is_ok();
    if let Err(e) = init_result {
        // Check if it's a missing init (which is OK if not required)
        let required_exports: Vec<&str> = args.exports.split(',').collect();
        if required_exports.contains(&"init") {
            anyhow::bail!("Module init failed: {}", e);
        }
        // Otherwise, init is optional - module is still valid
    }

    if args.verbose {
        println!("Module information:");
        println!("  Size: {} bytes", wasm_bytes.len());
        println!("  Has init: {}", init_ok);
        println!("  Ready: {}", host.is_ready());
        println!("  Supports hot-reload: {}", host.supports_hot_reload());
    }

    println!("Module is valid and compatible with Virtual PLC runtime");

    Ok(())
}

// =============================================================================
// SUBCOMMAND: simulate
// =============================================================================

fn cmd_simulate(args: SimulateArgs) -> Result<()> {
    use plc_runtime::io_image::ProcessData;

    info!(module = ?args.module, cycles = args.cycles, "Starting simulation");

    // Read module
    let module_bytes = std::fs::read(&args.module)
        .with_context(|| format!("Failed to read module: {:?}", args.module))?;

    // Check if it's WAT and convert if needed
    let wasm_bytes = if args.module.extension().map_or(false, |e| e == "wat") {
        wat::parse_bytes(&module_bytes)
            .with_context(|| "Failed to parse WAT")?
            .into_owned()
    } else {
        module_bytes
    };

    // Create Wasm host
    let cycle_time = Duration::from_millis(args.cycle_time_ms);
    let mut host = WasmtimeHost::new(cycle_time)
        .with_context(|| "Failed to create Wasm host")?;

    host.load_module(&wasm_bytes)
        .with_context(|| "Failed to load module")?;

    // Initialize
    host.init().with_context(|| "Module init() failed")?;

    // Parse initial digital inputs
    let initial_di = if args.digital_inputs.starts_with("0x") || args.digital_inputs.starts_with("0X") {
        u32::from_str_radix(&args.digital_inputs[2..], 16)
            .with_context(|| "Invalid hex value for digital_inputs")?
    } else {
        args.digital_inputs.parse::<u32>()
            .with_context(|| "Invalid value for digital_inputs")?
    };

    // Create process inputs using ProcessData
    let mut inputs = ProcessData::default();
    inputs.digital_inputs[0] = initial_di;

    println!("Simulating {} cycles with {}ms cycle time...\n", args.cycles, args.cycle_time_ms);

    if args.print_every > 0 {
        println!("{:>8} {:>12} {:>12} {:>12}",
            "Cycle", "DI (hex)", "DO (hex)", "AO[0]");
        println!("{:-<8} {:-<12} {:-<12} {:-<12}", "", "", "", "");
    }

    let start = std::time::Instant::now();

    for cycle in 0..args.cycles {
        // Execute step
        let outputs = host.step(&inputs)
            .with_context(|| format!("Cycle {} failed", cycle))?;

        // Print state periodically
        if args.print_every > 0 && (cycle % args.print_every == 0 || cycle == args.cycles - 1) {
            println!("{:>8} {:>12} {:>12} {:>12}",
                cycle,
                format!("0x{:08X}", inputs.digital_inputs[0]),
                format!("0x{:08X}", outputs.digital_outputs[0]),
                outputs.analog_outputs[0]
            );
        }

        // Simulate cycle delay (non-RT)
        std::thread::sleep(cycle_time);
    }

    let elapsed = start.elapsed();
    println!("\nSimulation complete:");
    println!("  Cycles: {}", args.cycles);
    println!("  Elapsed: {:.2}s", elapsed.as_secs_f64());
    println!("  Avg cycle: {:.2}ms", elapsed.as_millis() as f64 / args.cycles as f64);

    Ok(())
}

// =============================================================================
// SUBCOMMAND: diagnose
// =============================================================================

fn cmd_diagnose(args: DiagnoseArgs) -> Result<()> {
    use serde_json::json;

    // Collect diagnostic data
    let mut diag = DiagnosticReport::default();

    // OS information
    diag.os.platform = std::env::consts::OS.to_string();
    diag.os.arch = std::env::consts::ARCH.to_string();

    #[cfg(target_os = "linux")]
    {
        // Check for PREEMPT_RT
        if let Ok(version) = std::fs::read_to_string("/proc/version") {
            diag.os.preempt_rt = version.contains("PREEMPT_RT") || version.contains("PREEMPT RT");
            // Extract kernel version
            if let Some(ver) = version.split_whitespace().nth(2) {
                diag.os.kernel_version = Some(ver.to_string());
            }
        }

        // Check CPU isolation
        if let Ok(cmdline) = std::fs::read_to_string("/proc/cmdline") {
            diag.os.cpu_isolation = cmdline.contains("isolcpus");
            diag.os.nohz_full = cmdline.contains("nohz_full");
            diag.os.rcu_nocbs = cmdline.contains("rcu_nocbs");

            // Extract isolated CPUs if present
            if let Some(start) = cmdline.find("isolcpus=") {
                let rest = &cmdline[start + 9..];
                if let Some(end) = rest.find(|c: char| c.is_whitespace()) {
                    diag.os.isolated_cpus = Some(rest[..end].to_string());
                } else {
                    diag.os.isolated_cpus = Some(rest.to_string());
                }
            }
        }

        // Check RT scheduling capability
        diag.capabilities.rt_scheduling = unsafe {
            let result = libc::sched_setscheduler(
                0,
                libc::SCHED_FIFO,
                &libc::sched_param { sched_priority: 1 },
            );
            if result == 0 {
                libc::sched_setscheduler(
                    0,
                    libc::SCHED_OTHER,
                    &libc::sched_param { sched_priority: 0 },
                );
                true
            } else {
                false
            }
        };

        // Check memory locking capability
        diag.capabilities.memory_lock = unsafe {
            let result = libc::mlockall(libc::MCL_CURRENT | libc::MCL_FUTURE);
            if result == 0 {
                libc::munlockall();
                true
            } else {
                false
            }
        };

        // Check raw socket capability (for EtherCAT)
        diag.capabilities.raw_sockets = unsafe {
            let sock = libc::socket(libc::AF_PACKET, libc::SOCK_RAW, 0);
            if sock >= 0 {
                libc::close(sock);
                true
            } else {
                false
            }
        };

        // List network interfaces
        if let Ok(entries) = std::fs::read_dir("/sys/class/net") {
            for entry in entries.flatten() {
                if let Ok(name) = entry.file_name().into_string() {
                    if name != "lo" {
                        // Skip loopback
                        let mut iface = NetworkInterface {
                            name: name.clone(),
                            ..Default::default()
                        };

                        // Check if it's a physical interface
                        let device_path = format!("/sys/class/net/{}/device", name);
                        iface.is_physical = std::path::Path::new(&device_path).exists();

                        // Get driver info
                        let driver_path = format!("/sys/class/net/{}/device/driver", name);
                        if let Ok(driver_link) = std::fs::read_link(&driver_path) {
                            if let Some(driver_name) = driver_link.file_name() {
                                iface.driver = driver_name.to_string_lossy().to_string();
                            }
                        }

                        // Check operstate
                        let state_path = format!("/sys/class/net/{}/operstate", name);
                        if let Ok(state) = std::fs::read_to_string(&state_path) {
                            iface.state = state.trim().to_string();
                        }

                        // Recommended for EtherCAT if Intel i210/i350/i225
                        let recommended_drivers = ["igb", "igc", "e1000e"];
                        iface.ethercat_suitable = iface.is_physical
                            && recommended_drivers
                                .iter()
                                .any(|d| iface.driver.contains(d));

                        diag.network_interfaces.push(iface);
                    }
                }
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        diag.capabilities.rt_scheduling = false;
        diag.capabilities.memory_lock = false;
        diag.capabilities.raw_sockets = false;
    }

    // CPU info
    diag.cpu.count = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);

    // Timing test
    if !args.skip_timing_test {
        let target = Duration::from_micros(args.cycle_us);
        let iterations = (args.duration * 1_000_000) / args.cycle_us;

        let mut max_jitter_us = 0i64;
        let mut total_jitter_us = 0i64;
        let mut overruns = 0u64;

        let start = std::time::Instant::now();
        let mut next_wake = start;

        for _ in 0..iterations {
            next_wake += target;

            let now = std::time::Instant::now();
            if now < next_wake {
                std::thread::sleep(next_wake - now);
            }

            let actual = std::time::Instant::now();
            let jitter = actual.duration_since(next_wake);
            let jitter_us = jitter.as_micros() as i64;

            if jitter_us > max_jitter_us {
                max_jitter_us = jitter_us;
            }
            total_jitter_us += jitter_us;

            if jitter_us > args.cycle_us as i64 / 2 {
                overruns += 1;
            }
        }

        diag.timing.duration_secs = args.duration;
        diag.timing.target_cycle_us = args.cycle_us;
        diag.timing.iterations = iterations;
        diag.timing.max_jitter_us = max_jitter_us;
        diag.timing.avg_jitter_us = total_jitter_us / iterations as i64;
        diag.timing.overruns = overruns;
        diag.timing.ran_test = true;
    }

    // Generate recommendations
    if !diag.os.preempt_rt {
        diag.recommendations
            .push("Install PREEMPT_RT kernel for deterministic timing".to_string());
    }
    if !diag.os.cpu_isolation {
        diag.recommendations.push(
            "Configure CPU isolation (isolcpus, nohz_full, rcu_nocbs) for dedicated cores"
                .to_string(),
        );
    }
    if !diag.capabilities.rt_scheduling {
        diag.recommendations.push(
            "Run as root or grant CAP_SYS_NICE capability for real-time scheduling".to_string(),
        );
    }
    if !diag.capabilities.memory_lock {
        diag.recommendations.push(
            "Grant CAP_IPC_LOCK capability or increase RLIMIT_MEMLOCK for memory locking"
                .to_string(),
        );
    }
    if !diag.capabilities.raw_sockets {
        diag.recommendations.push(
            "Grant CAP_NET_RAW capability for EtherCAT raw socket access".to_string(),
        );
    }
    if diag.timing.ran_test {
        if diag.timing.max_jitter_us > args.cycle_us as i64 {
            diag.recommendations
                .push("Max jitter exceeds cycle time - consider longer cycle or RT tuning".to_string());
        }
        if diag.timing.overruns > 0 {
            diag.recommendations.push(format!(
                "{} timing overruns detected - reduce system load or increase cycle time",
                diag.timing.overruns
            ));
        }
    }

    // Check for suitable EtherCAT interface
    let has_ethercat_nic = diag.network_interfaces.iter().any(|i| i.ethercat_suitable);
    if !has_ethercat_nic && !diag.network_interfaces.is_empty() {
        diag.recommendations.push(
            "No recommended EtherCAT NIC found (Intel i210/i350/i225 preferred)".to_string(),
        );
    }

    // Output
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "os": {
                    "platform": diag.os.platform,
                    "arch": diag.os.arch,
                    "kernel_version": diag.os.kernel_version,
                    "preempt_rt": diag.os.preempt_rt,
                    "cpu_isolation": diag.os.cpu_isolation,
                    "isolated_cpus": diag.os.isolated_cpus,
                    "nohz_full": diag.os.nohz_full,
                    "rcu_nocbs": diag.os.rcu_nocbs,
                },
                "cpu": {
                    "count": diag.cpu.count,
                },
                "capabilities": {
                    "rt_scheduling": diag.capabilities.rt_scheduling,
                    "memory_lock": diag.capabilities.memory_lock,
                    "raw_sockets": diag.capabilities.raw_sockets,
                },
                "network_interfaces": diag.network_interfaces.iter().map(|i| json!({
                    "name": i.name,
                    "driver": i.driver,
                    "state": i.state,
                    "is_physical": i.is_physical,
                    "ethercat_suitable": i.ethercat_suitable,
                })).collect::<Vec<_>>(),
                "timing": if diag.timing.ran_test { Some(json!({
                    "duration_secs": diag.timing.duration_secs,
                    "target_cycle_us": diag.timing.target_cycle_us,
                    "iterations": diag.timing.iterations,
                    "max_jitter_us": diag.timing.max_jitter_us,
                    "avg_jitter_us": diag.timing.avg_jitter_us,
                    "overruns": diag.timing.overruns,
                })) } else { None },
                "recommendations": diag.recommendations,
            }))?
        );
    } else {
        println!("Virtual PLC System Diagnostics");
        println!("==============================\n");

        println!("Operating System:");
        println!("  Platform: {}", diag.os.platform);
        println!("  Architecture: {}", diag.os.arch);
        if let Some(ref ver) = diag.os.kernel_version {
            println!("  Kernel: {}", ver);
        }
        println!(
            "  PREEMPT_RT: {}",
            if diag.os.preempt_rt { "Yes" } else { "No" }
        );
        println!(
            "  CPU isolation: {}",
            if diag.os.cpu_isolation {
                format!(
                    "Yes ({})",
                    diag.os.isolated_cpus.as_deref().unwrap_or("unknown")
                )
            } else {
                "No".to_string()
            }
        );
        println!(
            "  nohz_full: {}",
            if diag.os.nohz_full { "Yes" } else { "No" }
        );
        println!(
            "  rcu_nocbs: {}",
            if diag.os.rcu_nocbs { "Yes" } else { "No" }
        );

        println!("\nCPU:");
        println!("  Available cores: {}", diag.cpu.count);

        println!("\nRuntime Capabilities:");
        println!(
            "  Real-time scheduling: {}",
            if diag.capabilities.rt_scheduling {
                "Available"
            } else {
                "Requires privileges"
            }
        );
        println!(
            "  Memory locking: {}",
            if diag.capabilities.memory_lock {
                "Available"
            } else {
                "Requires privileges"
            }
        );
        println!(
            "  Raw sockets: {}",
            if diag.capabilities.raw_sockets {
                "Available"
            } else {
                "Requires privileges"
            }
        );

        if !diag.network_interfaces.is_empty() {
            println!("\nNetwork Interfaces:");
            for iface in &diag.network_interfaces {
                let suitable = if iface.ethercat_suitable {
                    " [EtherCAT OK]"
                } else {
                    ""
                };
                println!(
                    "  {}: {} ({}){}",
                    iface.name, iface.driver, iface.state, suitable
                );
            }
        }

        if diag.timing.ran_test {
            println!(
                "\nTiming Test ({} seconds @ {} us cycle):",
                diag.timing.duration_secs, diag.timing.target_cycle_us
            );
            println!("  Iterations: {}", diag.timing.iterations);
            println!("  Max jitter: {} us", diag.timing.max_jitter_us);
            println!("  Avg jitter: {} us", diag.timing.avg_jitter_us);
            println!("  Overruns (>50% cycle): {}", diag.timing.overruns);
        }

        if !diag.recommendations.is_empty() {
            println!("\nRecommendations:");
            for rec in &diag.recommendations {
                println!("  - {}", rec);
            }
        } else {
            println!("\nSystem configuration looks good for real-time PLC operation.");
        }
    }

    Ok(())
}

// Diagnostic data structures
#[derive(Default)]
struct DiagnosticReport {
    os: OsInfo,
    cpu: CpuInfo,
    capabilities: Capabilities,
    network_interfaces: Vec<NetworkInterface>,
    timing: TimingTest,
    recommendations: Vec<String>,
}

#[derive(Default)]
struct OsInfo {
    platform: String,
    arch: String,
    kernel_version: Option<String>,
    preempt_rt: bool,
    cpu_isolation: bool,
    isolated_cpus: Option<String>,
    nohz_full: bool,
    rcu_nocbs: bool,
}

#[derive(Default)]
struct CpuInfo {
    count: usize,
}

#[derive(Default)]
struct Capabilities {
    rt_scheduling: bool,
    memory_lock: bool,
    raw_sockets: bool,
}

#[derive(Default)]
struct NetworkInterface {
    name: String,
    driver: String,
    state: String,
    is_physical: bool,
    ethercat_suitable: bool,
}

#[derive(Default)]
struct TimingTest {
    ran_test: bool,
    duration_secs: u64,
    target_cycle_us: u64,
    iterations: u64,
    max_jitter_us: i64,
    avg_jitter_us: i64,
    overruns: u64,
}

// =============================================================================
// DAEMON IMPLEMENTATION (moved from original)
// =============================================================================

/// Main daemon run loop.
fn run_daemon(
    config: &RuntimeConfig,
    signal_handler: &SignalHandler,
    diagnostics: &DiagnosticsCollector,
    max_cycles: u64,
) -> Result<()> {
    let metrics_http_export = config.metrics.http_export;
    let target_cycle_ns = u64::try_from(config.cycle_time.as_nanos()).unwrap_or(u64::MAX);

    // Start web UI server if HTTP export is enabled
    let state_updater = if metrics_http_export {
        let bind_addr: SocketAddr = format!("0.0.0.0:{}", config.metrics.http_port)
            .parse()
            .context("Invalid HTTP bind address")?;

        let web_config = WebUiConfig {
            bind_addr,
            enable_cors: true,
            static_dir: None,
            ws_channel_capacity: 256,
        };

        let server = WebUiServer::new(web_config);
        let updater = server.state_updater();

        // Create tokio runtime for async web server
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .context("Failed to create tokio runtime for web UI")?;

        // Spawn web server in background
        rt.spawn(async move {
            if let Err(e) = server.start().await {
                error!(error = %e, "Web UI server error");
            }
        });

        // Keep runtime alive by leaking it (server runs for daemon lifetime)
        std::mem::forget(rt);

        info!(addr = %bind_addr, "Web UI server started");
        Some(updater)
    } else {
        None
    };

    // Initialize fieldbus driver
    let mut fieldbus = create_fieldbus_driver(config)?;
    fieldbus.init().context("Failed to initialize fieldbus")?;
    diagnostics.state().set_fieldbus_connected(true);
    info!("Fieldbus driver initialized");

    // Create scheduler with appropriate engine
    let has_wasm = config.wasm_module.is_some();

    if has_wasm {
        let wasm_path = config
            .wasm_module
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("wasm_module required but not configured"))?;
        info!(?wasm_path, "Loading Wasm module");

        let wasm_bytes = std::fs::read(wasm_path)
            .with_context(|| format!("Failed to read Wasm module: {:?}", wasm_path))?;

        let engine = WasmtimeHost::from_runtime_config(config)
            .with_context(|| "Failed to create Wasmtime host")?;

        let mut scheduler = create_scheduler(engine, config);

        scheduler
            .engine
            .load_module(&wasm_bytes)
            .with_context(|| "Failed to load Wasm module")?;

        diagnostics.state().set_wasm_loaded(true);

        run_scheduler_loop(
            &mut scheduler,
            &mut fieldbus,
            signal_handler,
            diagnostics,
            max_cycles,
            metrics_http_export,
            target_cycle_ns,
            &config.fault_policy.fieldbus_failure,
            config.wasm_module.as_deref(),
            state_updater,
        )
    } else {
        info!("No Wasm module configured, using NullEngine");
        let engine = NullEngine::default();
        let mut scheduler = create_scheduler(engine, config);
        diagnostics.state().set_wasm_loaded(false);

        run_scheduler_loop(
            &mut scheduler,
            &mut fieldbus,
            signal_handler,
            diagnostics,
            max_cycles,
            metrics_http_export,
            target_cycle_ns,
            &config.fault_policy.fieldbus_failure,
            None, // NullEngine doesn't support hot-reload
            state_updater,
        )
    }
}

/// Create the appropriate fieldbus driver based on configuration.
fn create_fieldbus_driver(config: &RuntimeConfig) -> Result<Box<dyn FieldbusDriver>> {
    match config.fieldbus.driver {
        FieldbusDriverType::Simulated => {
            info!("Using simulated fieldbus driver");
            Ok(Box::new(SimulatedDriver::new()))
        }
        FieldbusDriverType::EtherCAT => {
            warn!("EtherCAT driver not fully implemented, falling back to simulated");
            Ok(Box::new(SimulatedDriver::new()))
        }
        FieldbusDriverType::ModbusTcp => {
            let modbus_cfg = config.fieldbus.modbus.as_ref().cloned().unwrap_or_default();

            let server_addr: std::net::SocketAddr = modbus_cfg
                .address
                .parse()
                .with_context(|| format!("Invalid Modbus server address: {}", modbus_cfg.address))?;

            let driver_config = ModbusTcpConfig {
                server_addr,
                unit_id: modbus_cfg.unit_id,
                connect_timeout: modbus_cfg.timeout,
                io_timeout: modbus_cfg.timeout,
                ..ModbusTcpConfig::default()
            };

            info!(
                server = %driver_config.server_addr,
                unit_id = driver_config.unit_id,
                "Using Modbus TCP driver"
            );
            Ok(Box::new(ModbusTcpDriver::with_config(driver_config)))
        }
    }
}

/// Create scheduler with the given logic engine.
fn create_scheduler<E: LogicEngine>(
    engine: E,
    config: &RuntimeConfig,
) -> Scheduler<E> {
    SchedulerBuilder::new(engine)
        .config(config.clone())
        .watchdog_timeout(config.watchdog_timeout)
        .build()
}

/// Run the scheduler main loop.
fn run_scheduler_loop<E: LogicEngine>(
    scheduler: &mut Scheduler<E>,
    fieldbus: &mut Box<dyn FieldbusDriver>,
    signal_handler: &SignalHandler,
    diagnostics: &DiagnosticsCollector,
    max_cycles: u64,
    metrics_http_export: bool,
    target_cycle_ns: u64,
    failure_policy: &plc_common::config::FieldbusFailurePolicy,
    wasm_module_path: Option<&std::path::Path>,
    state_updater: Option<StateUpdater>,
) -> Result<()> {
    scheduler
        .initialize()
        .context("Failed to initialize scheduler")?;
    info!("Scheduler initialized");

    // Initialize web UI session tracking
    if let Some(ref updater) = state_updater {
        updater.start_session();
        updater.set_runtime_state(RuntimeState::Init);
    }

    let _epoch_ticker = scheduler.engine.start_epoch_ticker();
    if _epoch_ticker.is_some() {
        info!("Epoch ticker started for Wasm timeout enforcement");
    }

    scheduler.start().context("Failed to start scheduler")?;
    info!(
        state = %scheduler.state(),
        "Scheduler started, entering main loop"
    );

    // Notify web UI of state change
    if let Some(ref updater) = state_updater {
        updater.set_runtime_state(RuntimeState::Run);
    }

    let mut cycles_run = 0u64;
    let mut consecutive_fb_failures = 0u32;
    let mut in_failure_streak = false;
    let mut recovery_cycles_remaining = 0u32;

    // Web UI update interval (every N cycles to avoid overhead)
    const WEB_UI_UPDATE_INTERVAL: u64 = 100;

    let shutdown_requested = wait_for_shutdown(signal_handler, std::time::Duration::from_millis(0));
    if shutdown_requested {
        info!("Shutdown already requested before entering main loop");
    }

    if !shutdown_requested {
        while scheduler.state() == RuntimeState::Run {
            if signal_handler.shutdown_requested() {
                info!("Shutdown signal received, stopping scheduler");
                break;
            }

            if signal_handler.take_reload_request() {
                if let Some(wasm_path) = wasm_module_path {
                    if scheduler.engine.supports_hot_reload() {
                        info!(?wasm_path, "Hot-reload requested, loading module");
                        match std::fs::read(wasm_path) {
                            Ok(wasm_bytes) => {
                                // Reload with memory preservation to maintain state
                                match scheduler.engine.reload_module(&wasm_bytes, true) {
                                    Ok(()) => {
                                        info!("Hot-reload successful, module updated");
                                        diagnostics.state().set_wasm_loaded(true);
                                    }
                                    Err(e) => {
                                        error!(error = %e, "Hot-reload failed, keeping previous module");
                                    }
                                }
                            }
                            Err(e) => {
                                error!(error = %e, ?wasm_path, "Failed to read Wasm module for hot-reload");
                            }
                        }
                    } else {
                        warn!("Hot-reload requested but engine does not support it");
                    }
                } else {
                    info!("Reload signal received but no Wasm module configured");
                }
            }

            if !in_failure_streak {
                let outputs = scheduler.io.read_outputs();
                let fb_outputs = plc_fieldbus::FieldbusOutputs {
                    digital: outputs.digital_outputs[0],
                    analog: outputs.analog_outputs,
                };
                fieldbus.set_outputs(&fb_outputs);
            }

            if let Err(e) = fieldbus.exchange() {
                if matches!(e, plc_common::error::PlcError::WkcThresholdExceeded { .. }) {
                    error!(error = %e, "WKC threshold exceeded - immediate fault");
                    diagnostics.state().set_fieldbus_connected(false);
                    // Record fault in web UI
                    if let Some(ref updater) = state_updater {
                        updater.record_fault("WKC threshold exceeded".to_string(), cycles_run);
                        updater.set_runtime_state(RuntimeState::Fault);
                    }
                    if let Err(fe) = scheduler.enter_fault("WKC threshold exceeded") {
                        warn!("Failed to enter fault state: {}", fe);
                    }
                    signal_handler.request_shutdown();
                    break;
                }

                consecutive_fb_failures += 1;
                in_failure_streak = true;
                recovery_cycles_remaining = 0;
                error!(
                    error = %e,
                    consecutive_failures = consecutive_fb_failures,
                    "Fieldbus exchange failed"
                );
                diagnostics.state().set_fieldbus_connected(false);

                let safe_outputs = plc_fieldbus::FieldbusOutputs {
                    digital: 0,
                    analog: [0; 16],
                };
                fieldbus.set_outputs(&safe_outputs);
                let _ = fieldbus.exchange();

                let max_failures = failure_policy.max_consecutive_failures;
                if max_failures > 0 && consecutive_fb_failures >= max_failures {
                    error!(
                        failures = consecutive_fb_failures,
                        threshold = max_failures,
                        "Maximum consecutive fieldbus failures reached, entering fault state"
                    );
                    // Record fault in web UI
                    if let Some(ref updater) = state_updater {
                        updater.record_fault("Fieldbus failure limit exceeded".to_string(), cycles_run);
                        updater.set_runtime_state(RuntimeState::Fault);
                    }
                    if let Err(e) = scheduler.enter_fault("Fieldbus failure limit exceeded") {
                        warn!("Failed to enter fault state: {}", e);
                    }
                    signal_handler.request_shutdown();
                    break;
                }
            } else {
                if in_failure_streak {
                    if recovery_cycles_remaining == 0 {
                        recovery_cycles_remaining = failure_policy.recovery_grace_cycles;
                    }

                    if recovery_cycles_remaining > 0 {
                        recovery_cycles_remaining -= 1;
                    }

                    if recovery_cycles_remaining == 0 {
                        info!(
                            previous_failures = consecutive_fb_failures,
                            grace_cycles = failure_policy.recovery_grace_cycles,
                            "Fieldbus communication recovered"
                        );
                        in_failure_streak = false;
                        consecutive_fb_failures = 0;

                        let outputs = scheduler.io.read_outputs();
                        let fb_outputs = plc_fieldbus::FieldbusOutputs {
                            digital: outputs.digital_outputs[0],
                            analog: outputs.analog_outputs,
                        };
                        fieldbus.set_outputs(&fb_outputs);
                        let _ = fieldbus.exchange();
                    }
                }
                diagnostics.state().set_fieldbus_connected(true);
            }

            {
                let fb_inputs = fieldbus.get_inputs();
                scheduler.io.write_inputs(|data| {
                    data.digital_inputs[0] = fb_inputs.digital;
                    data.analog_inputs = fb_inputs.analog;
                });
            }

            match scheduler.run_cycle() {
                Ok(result) => {
                    diagnostics
                        .state()
                        .record_cycle(result.execution_time, result.overrun);

                    if result.overrun {
                        warn!(
                            cycle = result.cycle_count,
                            execution_us = result.execution_time.as_micros(),
                            "Cycle overrun detected"
                        );
                    }
                }
                Err(e) => {
                    error!("Cycle execution failed: {}", e);
                    signal_handler.request_shutdown();
                    break;
                }
            }

            cycles_run += 1;
            if max_cycles > 0 && cycles_run >= max_cycles {
                info!(cycles = cycles_run, "Maximum cycle count reached");
                signal_handler.request_shutdown();
                break;
            }

            if cycles_run % 10000 == 0 {
                let metrics = scheduler.metrics();
                info!(
                    cycles = cycles_run,
                    avg_us = metrics.mean().map(|d| d.as_micros()).unwrap_or(0),
                    max_us = metrics.max().map(|d| d.as_micros()).unwrap_or(0),
                    overruns = diagnostics.state().overrun_count(),
                    "Periodic status"
                );
            }

            // Update web UI periodically (not every cycle to avoid overhead)
            if let Some(ref updater) = state_updater {
                if cycles_run % WEB_UI_UPDATE_INTERVAL == 0 {
                    // Update I/O state
                    let inputs = scheduler.io.read_inputs();
                    let outputs = scheduler.io.read_outputs();
                    updater.update_io_raw(
                        inputs.digital_inputs[0],
                        outputs.digital_outputs[0],
                        &inputs.analog_inputs,
                        &outputs.analog_outputs,
                    );

                    // Update metrics
                    let metrics = scheduler.metrics();
                    updater.update_metrics_raw(
                        cycles_run,
                        metrics.min().map(|d| d.as_nanos() as u64).unwrap_or(0),
                        metrics.max().map(|d| d.as_nanos() as u64).unwrap_or(0),
                        metrics.mean().map(|d| d.as_nanos() as u64).unwrap_or(0),
                        target_cycle_ns,
                        diagnostics.state().overrun_count() as u64,
                    );
                }
            }
        }
    }

    // Notify web UI of shutdown
    if let Some(ref updater) = state_updater {
        updater.set_runtime_state(scheduler.state());
    }

    info!("Shutting down...");

    if let Err(e) = scheduler.stop() {
        warn!("Scheduler stop failed: {}", e);
    }

    if let Err(e) = fieldbus.shutdown() {
        warn!("Fieldbus shutdown failed: {}", e);
    }

    let snapshot = diagnostics.snapshot(scheduler.state(), scheduler.metrics());
    if metrics_http_export {
        let _ = format_prometheus_metrics(&snapshot, target_cycle_ns);
    }
    info!(
        total_cycles = snapshot.cycle_count,
        overruns = snapshot.overrun_count,
        signals = signal_handler.state().signal_count(),
        uptime_secs = snapshot.uptime.as_secs(),
        final_state = %snapshot.state,
        "Daemon shutdown complete"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_run_subcommand() {
        let cli = Cli::parse_from(["plc-daemon", "run", "--simulated"]);
        match cli.command {
            Commands::Run(args) => assert!(args.simulated),
            _ => panic!("Expected Run command"),
        }
    }

    #[test]
    fn test_cli_compile_subcommand() {
        let cli = Cli::parse_from(["plc-daemon", "compile", "test.st", "-o", "test.wasm"]);
        match cli.command {
            Commands::Compile(args) => {
                assert_eq!(args.input, PathBuf::from("test.st"));
                assert_eq!(args.output, Some(PathBuf::from("test.wasm")));
            }
            _ => panic!("Expected Compile command"),
        }
    }

    #[test]
    fn test_cli_validate_subcommand() {
        let cli = Cli::parse_from(["plc-daemon", "validate", "module.wasm", "--verbose"]);
        match cli.command {
            Commands::Validate(args) => {
                assert_eq!(args.module, PathBuf::from("module.wasm"));
                assert!(args.verbose);
            }
            _ => panic!("Expected Validate command"),
        }
    }

    #[test]
    fn test_cli_simulate_subcommand() {
        let cli = Cli::parse_from(["plc-daemon", "simulate", "test.wasm", "-n", "50"]);
        match cli.command {
            Commands::Simulate(args) => {
                assert_eq!(args.module, PathBuf::from("test.wasm"));
                assert_eq!(args.cycles, 50);
            }
            _ => panic!("Expected Simulate command"),
        }
    }

    #[test]
    fn test_cli_diagnose_subcommand() {
        let cli = Cli::parse_from(["plc-daemon", "diagnose", "--duration", "10"]);
        match cli.command {
            Commands::Diagnose(args) => {
                assert_eq!(args.duration, 10);
            }
            _ => panic!("Expected Diagnose command"),
        }
    }
}
