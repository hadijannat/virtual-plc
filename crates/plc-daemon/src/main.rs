//! PLC daemon entry point.
//!
//! Integrates the scheduler, fieldbus driver, and Wasm logic engine
//! into a complete runtime with signal handling and diagnostics.

mod diagnostics;
mod signals;

use anyhow::{Context, Result};
use clap::Parser;
use plc_common::config::{FieldbusDriver as FieldbusDriverType, RuntimeConfig};
use plc_common::state::RuntimeState;
use plc_fieldbus::{FieldbusDriver, SimulatedDriver};
use plc_runtime::scheduler::{Scheduler, SchedulerBuilder};
use plc_runtime::wasm_host::{NullEngine, WasmtimeHost};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{error, info, warn};

use crate::diagnostics::{format_prometheus_metrics, DiagnosticsCollector, DiagnosticsState};
use crate::signals::{wait_for_shutdown, SignalHandler};

/// PLC daemon command-line arguments.
#[derive(Parser, Debug)]
#[command(
    name = "plc-daemon",
    about = "Virtual PLC daemon - real-time industrial control runtime",
    version,
    long_about = None
)]
struct Args {
    /// Path to a runtime configuration file (TOML).
    #[arg(long, short = 'c', value_name = "FILE")]
    config: Option<PathBuf>,

    /// Path to the Wasm logic module (overrides config file).
    #[arg(long, short = 'w', value_name = "FILE")]
    wasm_module: Option<PathBuf>,

    /// Run in simulated mode (no real fieldbus).
    #[arg(long, short = 's')]
    simulated: bool,

    /// Maximum cycles to run (0 = infinite).
    #[arg(long, default_value = "0")]
    max_cycles: u64,

    /// Log level (trace, debug, info, warn, error).
    #[arg(long, short = 'l', default_value = "info")]
    log_level: String,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize logging
    init_logging(&args.log_level);

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

/// Initialize logging with the specified log level.
fn init_logging(level: &str) {
    let filter = format!(
        "plc_daemon={},plc_runtime={},plc_fieldbus={},plc_common={}",
        level, level, level, level
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

/// Load configuration from file or use defaults.
///
/// Resolution priority (first existing file wins):
/// 1. Command-line `--config` argument
/// 2. `PLC_CONFIG_PATH` environment variable
/// 3. `/etc/plc/config.toml` (system path)
/// 4. `config/default.toml` (local development)
/// 5. Built-in defaults
fn load_config(args: &Args) -> Result<RuntimeConfig> {
    // 1. Command-line argument (highest priority)
    if let Some(config_path) = &args.config {
        info!(?config_path, "Loading config from command-line argument");
        return RuntimeConfig::from_file(config_path)
            .with_context(|| format!("Failed to load config from {:?}", config_path));
    }

    // 2. Environment variable
    if let Ok(env_path) = std::env::var("PLC_CONFIG_PATH") {
        let config_path = PathBuf::from(&env_path);
        if config_path.exists() {
            info!(?config_path, "Loading config from PLC_CONFIG_PATH");
            return RuntimeConfig::from_file(&config_path).with_context(|| {
                format!("Failed to load config from PLC_CONFIG_PATH={:?}", env_path)
            });
        }
        warn!(
            path = %env_path,
            "PLC_CONFIG_PATH set but file does not exist, checking other locations"
        );
    }

    // 3. System path
    let system_path = PathBuf::from("/etc/plc/config.toml");
    if system_path.exists() {
        info!(?system_path, "Loading config from system path");
        return RuntimeConfig::from_file(&system_path)
            .with_context(|| format!("Failed to load config from {:?}", system_path));
    }

    // 4. Local development path
    let local_path = PathBuf::from("config/default.toml");
    if local_path.exists() {
        info!(?local_path, "Loading config from local path");
        return RuntimeConfig::from_file(&local_path)
            .with_context(|| format!("Failed to load config from {:?}", local_path));
    }

    // 5. Built-in defaults
    info!("No config file found, using built-in defaults");
    Ok(RuntimeConfig::default())
}

/// Main daemon run loop.
fn run_daemon(
    config: &RuntimeConfig,
    signal_handler: &SignalHandler,
    diagnostics: &DiagnosticsCollector,
    max_cycles: u64,
) -> Result<()> {
    let metrics_http_export = config.metrics.http_export;
    let target_cycle_ns = u64::try_from(config.cycle_time.as_nanos()).unwrap_or(u64::MAX);

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

        // Load the Wasm module
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
            // EtherCAT requires additional setup
            warn!("EtherCAT driver not fully implemented, falling back to simulated");
            Ok(Box::new(SimulatedDriver::new()))
        }
        FieldbusDriverType::ModbusTcp => {
            warn!("Modbus TCP driver not implemented, falling back to simulated");
            Ok(Box::new(SimulatedDriver::new()))
        }
    }
}

/// Create scheduler with the given logic engine.
fn create_scheduler<E: plc_runtime::wasm_host::LogicEngine>(
    engine: E,
    config: &RuntimeConfig,
) -> Scheduler<E> {
    SchedulerBuilder::new(engine)
        .config(config.clone())
        .watchdog_timeout(config.watchdog_timeout)
        .build()
}

/// Run the scheduler main loop.
fn run_scheduler_loop<E: plc_runtime::wasm_host::LogicEngine>(
    scheduler: &mut Scheduler<E>,
    fieldbus: &mut Box<dyn FieldbusDriver>,
    signal_handler: &SignalHandler,
    diagnostics: &DiagnosticsCollector,
    max_cycles: u64,
    metrics_http_export: bool,
    target_cycle_ns: u64,
) -> Result<()> {
    // Initialize scheduler
    scheduler
        .initialize()
        .context("Failed to initialize scheduler")?;
    info!("Scheduler initialized");

    // Start epoch ticker for Wasm timeout enforcement
    // The ticker must be kept alive for the duration of the main loop
    let _epoch_ticker = scheduler.engine.start_epoch_ticker();
    if _epoch_ticker.is_some() {
        info!("Epoch ticker started for Wasm timeout enforcement");
    }

    // Start cyclic execution
    scheduler.start().context("Failed to start scheduler")?;
    info!(
        state = %scheduler.state(),
        "Scheduler started, entering main loop"
    );

    let mut cycles_run = 0u64;
    let mut consecutive_fb_failures = 0u32;
    let mut in_failure_streak = false;
    const MAX_CONSECUTIVE_FB_FAILURES: u32 = 3;

    let shutdown_requested = wait_for_shutdown(signal_handler, std::time::Duration::from_millis(0));
    if shutdown_requested {
        info!("Shutdown already requested before entering main loop");
    }

    if !shutdown_requested {
        while scheduler.state() == RuntimeState::Run {
            // Check for shutdown signal
            if signal_handler.shutdown_requested() {
                info!("Shutdown signal received, stopping scheduler");
                break;
            }

            // Check for reload signal (config reload)
            if signal_handler.take_reload_request() {
                info!("Reload signal received (config reload not yet implemented)");
            }

            // Copy outputs from scheduler to fieldbus before exchange
            // Skip if in failure streak - keep safe outputs latched
            if !in_failure_streak {
                let outputs = scheduler.io.read_outputs();
                let fb_outputs = plc_fieldbus::FieldbusOutputs {
                    digital: outputs.digital_outputs[0],
                    analog: outputs.analog_outputs,
                };
                fieldbus.set_outputs(&fb_outputs);
            }

            // Perform fieldbus exchange
            if let Err(e) = fieldbus.exchange() {
                consecutive_fb_failures += 1;
                in_failure_streak = true;
                error!(
                    error = %e,
                    consecutive_failures = consecutive_fb_failures,
                    "Fieldbus exchange failed"
                );
                diagnostics.state().set_fieldbus_connected(false);

                // Set safe outputs and attempt to send them
                let safe_outputs = plc_fieldbus::FieldbusOutputs {
                    digital: 0,
                    analog: [0; 16],
                };
                fieldbus.set_outputs(&safe_outputs);
                // Best-effort attempt to push safe outputs on the wire
                let _ = fieldbus.exchange();

                // Enter fault state after too many consecutive failures
                if consecutive_fb_failures >= MAX_CONSECUTIVE_FB_FAILURES {
                    error!(
                        failures = consecutive_fb_failures,
                        "Maximum consecutive fieldbus failures reached, entering fault state"
                    );
                    // Enter FAULT state (not SAFE_STOP) before exiting
                    if let Err(e) = scheduler.enter_fault("Fieldbus failure limit exceeded") {
                        warn!("Failed to enter fault state: {}", e);
                    }
                    signal_handler.request_shutdown();
                    break;
                }
            } else {
                // Recovery path: reset failure tracking on successful exchange
                if in_failure_streak {
                    info!(
                        previous_failures = consecutive_fb_failures,
                        "Fieldbus communication recovered"
                    );
                    // Clear streak FIRST so next iteration uses logic outputs
                    in_failure_streak = false;
                    consecutive_fb_failures = 0;

                    // Immediately copy fresh logic outputs and exchange again
                    // to avoid one-cycle delay in output recovery
                    let outputs = scheduler.io.read_outputs();
                    let fb_outputs = plc_fieldbus::FieldbusOutputs {
                        digital: outputs.digital_outputs[0],
                        analog: outputs.analog_outputs,
                    };
                    fieldbus.set_outputs(&fb_outputs);
                    // Best-effort to get outputs out immediately on recovery
                    let _ = fieldbus.exchange();
                }
                diagnostics.state().set_fieldbus_connected(true);
            }

            // Copy inputs from fieldbus to scheduler after exchange
            {
                let fb_inputs = fieldbus.get_inputs();
                scheduler.io.write_inputs(|data| {
                    data.digital_inputs[0] = fb_inputs.digital;
                    data.analog_inputs = fb_inputs.analog;
                });
            }

            // Run one PLC cycle
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

            // Check cycle limit
            cycles_run += 1;
            if max_cycles > 0 && cycles_run >= max_cycles {
                info!(cycles = cycles_run, "Maximum cycle count reached");
                signal_handler.request_shutdown();
                break;
            }

            // Periodic status logging (every 10000 cycles)
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
        }
    }

    // Graceful shutdown
    info!("Shutting down...");

    if let Err(e) = scheduler.stop() {
        warn!("Scheduler stop failed: {}", e);
    }

    if let Err(e) = fieldbus.shutdown() {
        warn!("Fieldbus shutdown failed: {}", e);
    }

    // Final statistics
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
    fn test_args_parsing() {
        let args = Args::parse_from(["plc-daemon", "--simulated"]);
        assert!(args.simulated);
        assert!(args.config.is_none());
    }

    #[test]
    fn test_args_with_config() {
        let args = Args::parse_from(["plc-daemon", "-c", "test.toml", "-w", "program.wasm"]);
        assert_eq!(args.config, Some(PathBuf::from("test.toml")));
        assert_eq!(args.wasm_module, Some(PathBuf::from("program.wasm")));
    }

    #[test]
    fn test_default_config() {
        // Should succeed with defaults even without config file
        let config = RuntimeConfig::default();
        assert_eq!(config.cycle_time.as_millis(), 1);
    }
}
