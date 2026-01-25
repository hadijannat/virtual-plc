//! WebAssembly host integration for PLC logic execution.
//!
//! This module provides a Wasmtime-based implementation of the [`LogicEngine`] trait,
//! allowing PLC programs compiled to WebAssembly to run in a sandboxed environment.
//!
//! # Features
//!
//! - **Epoch interruption**: Allows the host to interrupt long-running Wasm code
//!   for deterministic cycle timing
//! - **NaN canonicalization**: Ensures floating-point operations are deterministic
//! - **Cranelift JIT**: Fast compilation with optimizations for real-time execution
//!
//! # Usage
//!
//! ```ignore
//! let host = WasmtimeHost::new(cycle_time)?;
//! host.load_module(&wasm_bytes)?;
//! host.init()?;
//!
//! loop {
//!     host.step()?;
//! }
//! ```

use crate::io_image::ProcessData;
use crate::wasm_imports::{register_host_functions, HostState};
use crate::wasm_memory::{
    copy_inputs_to_wasm, copy_outputs_from_wasm, write_system_info, WasmSystemInfo,
};
use anyhow::{anyhow, Context, Result};
use plc_common::error::{PlcError, PlcResult};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tracing::{debug, info, trace, warn};
use wasmtime::{
    Config, Engine, Instance, Linker, Memory, Module, OptLevel, Store, Trap, TypedFunc,
};

/// Logic engine trait for swappable Wasm runtimes.
///
/// This trait allows the runtime to work with different Wasm engines
/// (Wasmtime, WAMR, wasm3, etc.) through a common interface.
pub trait LogicEngine: Send {
    /// Initialize the logic engine.
    ///
    /// Called once before the first cycle. Should call the Wasm module's
    /// initialization function if present.
    fn init(&mut self) -> PlcResult<()>;

    /// Execute one scan cycle.
    ///
    /// Copies inputs to Wasm memory, calls the step function,
    /// then copies outputs back. Must complete within cycle time.
    fn step(&mut self, inputs: &ProcessData) -> PlcResult<ProcessData>;

    /// Handle a fault condition.
    ///
    /// Called when a fault is detected. Should put outputs in a safe state.
    fn fault(&mut self) -> PlcResult<()>;

    /// Check if the engine is ready to execute.
    fn is_ready(&self) -> bool;

    /// Hot-reload a new Wasm module, preserving state where possible.
    ///
    /// This method allows updating the logic program without stopping I/O.
    /// It should be called at cycle boundaries (after outputs, before next inputs)
    /// to ensure consistent state.
    ///
    /// # State Migration
    ///
    /// The implementation may attempt to migrate state from the old module to
    /// the new one. The `preserve_memory` flag controls this behavior:
    /// - `true`: Copy linear memory from old to new module (if sizes are compatible)
    /// - `false`: Start with fresh memory (new module's initialization)
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The new module fails to compile
    /// - The new module is missing required exports (`step`, `memory`)
    /// - State migration fails (memory size incompatible when `preserve_memory` is true)
    ///
    /// On error, the old module remains active.
    ///
    /// The default implementation returns `Err(PlcError::Config)` indicating
    /// hot-reload is not supported.
    fn reload_module(&mut self, _wasm_bytes: &[u8], _preserve_memory: bool) -> PlcResult<()> {
        Err(PlcError::Config(
            "Hot-reload not supported by this engine".into(),
        ))
    }

    /// Check if the engine supports hot-reload.
    ///
    /// Returns `true` if `reload_module` is implemented and can be called.
    fn supports_hot_reload(&self) -> bool {
        false
    }

    /// Start the epoch ticker for timeout enforcement.
    ///
    /// Returns an `EpochTicker` handle that must be kept alive for the ticker
    /// to continue running. Dropping the handle stops the ticker.
    ///
    /// The default implementation returns `None` (no epoch support).
    fn start_epoch_ticker(&self) -> Option<EpochTicker> {
        None
    }
}

/// Handle for an epoch ticker thread.
///
/// The ticker runs in a background thread and increments the engine's epoch
/// counter at a fixed interval. Dropping this handle stops the ticker.
pub struct EpochTicker {
    stop_flag: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl EpochTicker {
    /// Create and start a new epoch ticker.
    ///
    /// The `tick_fn` is called at the specified `interval` until the ticker is stopped.
    ///
    /// # Errors
    ///
    /// Returns an error if the epoch ticker thread fails to spawn.
    pub fn new<F>(interval: Duration, tick_fn: F) -> Result<Self, PlcError>
    where
        F: Fn() + Send + 'static,
    {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_flag_clone = Arc::clone(&stop_flag);

        let handle = thread::Builder::new()
            .name("epoch-ticker".into())
            .spawn(move || {
                debug!("Epoch ticker thread started, interval: {:?}", interval);
                while !stop_flag_clone.load(Ordering::Acquire) {
                    tick_fn();
                    thread::sleep(interval);
                }
                debug!("Epoch ticker thread stopped");
            })
            .map_err(|e| PlcError::Config(format!("Failed to spawn epoch ticker: {}", e)))?;

        Ok(Self {
            stop_flag,
            handle: Some(handle),
        })
    }

    /// Stop the ticker and wait for the thread to finish.
    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for EpochTicker {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Wasmtime-based logic engine with epoch interruption support.
pub struct WasmtimeHost {
    /// Wasmtime engine with epoch support.
    engine: Engine,
    /// Store containing host state and instance.
    store: Store<HostState>,
    /// Linker with host functions.
    linker: Linker<HostState>,
    /// Compiled module (set after load_module).
    module: Option<Module>,
    /// Instantiated module.
    instance: Option<Instance>,
    /// Reference to Wasm memory.
    memory: Option<Memory>,
    /// Cached step function.
    step_fn: Option<TypedFunc<(), ()>>,
    /// Cached init function (optional).
    init_fn: Option<TypedFunc<(), ()>>,
    /// Cached fault function (optional).
    fault_fn: Option<TypedFunc<(), ()>>,
    /// Epoch counter for timeout.
    epoch_counter: Arc<AtomicU64>,
    /// Maximum epochs per cycle (timeout control).
    max_epochs_per_cycle: u64,
    /// Cycle time in nanoseconds (u64 to prevent overflow for cycles > 4.29s).
    cycle_time_ns: u64,
    /// Whether the engine has been initialized.
    initialized: bool,
    /// Local copy of process data for step().
    process_data: ProcessData,
    /// Whether fuel-based execution budgeting is enabled.
    use_fuel: bool,
    /// Fuel units to grant per cycle.
    fuel_per_cycle: u64,
}

impl std::fmt::Debug for WasmtimeHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmtimeHost")
            .field("module_loaded", &self.module.is_some())
            .field("initialized", &self.initialized)
            .field("cycle_time_ns", &self.cycle_time_ns)
            .finish()
    }
}

impl WasmtimeHost {
    /// Create a new Wasmtime host with the given cycle time.
    pub fn new(cycle_time: Duration) -> Result<Self> {
        Self::with_config(cycle_time, WasmtimeConfig::default())
    }

    /// Create a Wasmtime host from RuntimeConfig.
    ///
    /// Reads Wasm limits from the config's `wasm` section instead of using defaults.
    pub fn from_runtime_config(config: &plc_common::config::RuntimeConfig) -> Result<Self> {
        let wasm_config = WasmtimeConfig {
            opt_level: OptLevel::Speed,
            max_memory_bytes: config.wasm.max_memory_bytes,
            max_table_elements: config.wasm.max_table_elements,
            enable_simd: config.wasm.enable_simd,
            deterministic: config.wasm.deterministic,
            use_fuel: config.wasm.use_fuel,
            fuel_per_cycle: config.wasm.fuel_per_cycle,
        };

        Self::with_config_and_epochs(
            config.cycle_time,
            wasm_config,
            config.wasm.max_epochs_per_cycle,
        )
    }

    /// Create a new Wasmtime host with custom configuration.
    pub fn with_config(cycle_time: Duration, wasm_config: WasmtimeConfig) -> Result<Self> {
        // Calculate epochs per cycle based on cycle time
        // Rough estimate: 1 epoch ≈ 10µs of execution
        let max_epochs_per_cycle = (cycle_time.as_micros() as u64 / 10).max(100);
        Self::with_config_and_epochs(cycle_time, wasm_config, max_epochs_per_cycle)
    }

    /// Create a new Wasmtime host with explicit epoch configuration.
    fn with_config_and_epochs(
        cycle_time: Duration,
        wasm_config: WasmtimeConfig,
        max_epochs_per_cycle: u64,
    ) -> Result<Self> {
        // Configure Wasmtime for real-time PLC execution
        let mut config = Config::new();

        // Enable epoch interruption for timeout control
        config.epoch_interruption(true);

        // Set optimization level
        config.cranelift_opt_level(wasm_config.opt_level);

        // Configure Wasm features for PLC use
        config.wasm_threads(false);
        config.wasm_simd(wasm_config.enable_simd);
        // Relaxed SIMD requires SIMD to be enabled, but must be disabled in
        // deterministic mode as it allows implementation-specific behavior
        config.wasm_relaxed_simd(wasm_config.enable_simd && !wasm_config.deterministic);

        // Apply deterministic mode settings if enabled
        if wasm_config.deterministic {
            // Disable features that can introduce non-determinism:
            // - Reference types (externref/funcref) can have implementation-specific behavior
            config.wasm_reference_types(false);
            // - Bulk memory ops may have platform-specific edge cases
            config.wasm_bulk_memory(false);
            // - Multi-value returns add complexity
            config.wasm_multi_value(false);
            // - Function references can introduce non-determinism
            config.wasm_function_references(false);
            // - Tail calls can have stack behavior differences
            config.wasm_tail_call(false);

            debug!("Deterministic mode enabled: restricted Wasm feature set");
        }

        // Enable fuel-based execution budgeting if configured
        if wasm_config.use_fuel {
            config.consume_fuel(true);
            debug!(
                fuel_per_cycle = wasm_config.fuel_per_cycle,
                "Fuel-based execution budgeting enabled"
            );
        }

        // Create engine
        let engine = Engine::new(&config).context("Failed to create Wasmtime engine")?;

        // Create store with host state including resource limits
        // Use saturating conversion to handle extreme values safely
        let cycle_time_ns = u64::try_from(cycle_time.as_nanos()).unwrap_or(u64::MAX);
        let host_state = HostState::with_limits(
            cycle_time_ns,
            wasm_config.max_memory_bytes,
            wasm_config.max_table_elements,
        );

        let mut store = Store::new(&engine, host_state);

        // Enable the resource limiter. HostState implements ResourceLimiter,
        // so any attempt by the Wasm module to grow memory beyond max_memory_bytes
        // or tables beyond max_table_elements will result in a trap.
        store.limiter(|state| state);

        // Create linker and register host functions
        let mut linker = Linker::new(&engine);
        register_host_functions(&mut linker).context("Failed to register host functions")?;

        info!(
            cycle_time_ns,
            max_epochs_per_cycle,
            max_memory_bytes = wasm_config.max_memory_bytes,
            max_table_elements = wasm_config.max_table_elements,
            enable_simd = wasm_config.enable_simd,
            deterministic = wasm_config.deterministic,
            use_fuel = wasm_config.use_fuel,
            fuel_per_cycle = wasm_config.fuel_per_cycle,
            "WasmtimeHost created"
        );

        Ok(Self {
            engine,
            store,
            linker,
            module: None,
            instance: None,
            memory: None,
            step_fn: None,
            init_fn: None,
            fault_fn: None,
            epoch_counter: Arc::new(AtomicU64::new(0)),
            max_epochs_per_cycle,
            cycle_time_ns,
            initialized: false,
            process_data: ProcessData::default(),
            use_fuel: wasm_config.use_fuel,
            fuel_per_cycle: wasm_config.fuel_per_cycle,
        })
    }

    /// Load a Wasm module from bytes.
    pub fn load_module(&mut self, wasm_bytes: &[u8]) -> Result<()> {
        let module =
            Module::new(&self.engine, wasm_bytes).context("Failed to compile Wasm module")?;

        info!(
            exports = ?module.exports().map(|e| e.name()).collect::<Vec<_>>(),
            "Wasm module compiled"
        );

        self.module = Some(module);
        self.instance = None;
        self.memory = None;
        self.step_fn = None;
        self.init_fn = None;
        self.fault_fn = None;
        self.initialized = false;

        Ok(())
    }

    /// Load a Wasm module from WAT text format.
    pub fn load_wat(&mut self, wat: &str) -> Result<()> {
        let wasm_bytes = wat::parse_str(wat).context("Failed to parse WAT")?;
        self.load_module(&wasm_bytes)
    }

    /// Instantiate the loaded module.
    fn instantiate(&mut self) -> Result<()> {
        let module = self
            .module
            .as_ref()
            .ok_or_else(|| anyhow!("No module loaded"))?;

        // Ensure fuel is available before instantiation (start functions may run)
        if self.use_fuel {
            self.store
                .set_fuel(self.fuel_per_cycle)
                .map_err(|e| anyhow!("Failed to set fuel: {}", e))?;
        }

        // Instantiate
        let instance = self
            .linker
            .instantiate(&mut self.store, module)
            .context("Failed to instantiate module")?;

        // Get memory export
        let memory = instance
            .get_memory(&mut self.store, "memory")
            .ok_or_else(|| anyhow!("Module must export 'memory'"))?;

        // Set memory in host state
        self.store.data_mut().set_memory(memory);

        // Get required step function
        let step_fn: TypedFunc<(), ()> = instance
            .get_typed_func(&mut self.store, "step")
            .context("Module must export 'step' function")?;

        // Get optional init function
        let init_fn = instance.get_typed_func(&mut self.store, "init").ok();

        // Get optional fault function
        let fault_fn = instance.get_typed_func(&mut self.store, "fault").ok();

        debug!(
            has_init = init_fn.is_some(),
            has_fault = fault_fn.is_some(),
            memory_pages = memory.size(&self.store),
            "Module instantiated"
        );

        self.instance = Some(instance);
        self.memory = Some(memory);
        self.step_fn = Some(step_fn);
        self.init_fn = init_fn;
        self.fault_fn = fault_fn;

        Ok(())
    }

    /// Increment the epoch counter (call from timer thread).
    ///
    /// This increments both the Wasmtime engine's epoch and our local counter.
    /// The local counter is used to calculate deadlines for cycle timeouts.
    pub fn increment_epoch(&self) {
        self.engine.increment_epoch();
        self.epoch_counter.fetch_add(1, Ordering::Relaxed);
    }

    /// Get a handle to the epoch counter for external control.
    pub fn epoch_handle(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.epoch_counter)
    }

    /// Set the epoch deadline for the next cycle.
    fn set_epoch_deadline(&mut self) {
        let current = self.epoch_counter.load(Ordering::Relaxed);
        self.store
            .set_epoch_deadline(current + self.max_epochs_per_cycle);
    }

    /// Ensure fuel is set if fuel-based budgeting is enabled.
    ///
    /// This must be called before any Wasm execution (init, step, fault)
    /// to prevent immediate traps when the store starts with 0 fuel.
    fn ensure_fuel(&mut self) -> PlcResult<()> {
        if self.use_fuel {
            self.store
                .set_fuel(self.fuel_per_cycle)
                .map_err(|e| PlcError::Config(format!("Failed to set fuel: {e}")))?;
        }
        Ok(())
    }
}

impl LogicEngine for WasmtimeHost {
    fn init(&mut self) -> PlcResult<()> {
        if self.module.is_none() {
            return Err(PlcError::Config("No Wasm module loaded".into()));
        }

        // Instantiate if not already done
        if self.instance.is_none() {
            self.instantiate()
                .map_err(|e| PlcError::Config(e.to_string()))?;
        }

        // Call init function if present
        let has_init = self.init_fn.is_some();
        if has_init {
            self.set_epoch_deadline();
            self.ensure_fuel()?;
            // Get reference after set_epoch_deadline
            if let Some(init_fn) = &self.init_fn {
                init_fn
                    .call(&mut self.store, ())
                    .map_err(|e| PlcError::WasmTrap(format!("init() failed: {e}")))?;
            }
        }

        // Write initial system info
        let cycle_time_ns = self.cycle_time_ns;
        if let Some(memory) = self.memory {
            let data = memory.data_mut(&mut self.store);
            let sys_info = WasmSystemInfo {
                cycle_time_ns,
                flags: WasmSystemInfo::FLAG_FIRST_CYCLE,
            };
            write_system_info(data, &sys_info);
        }

        self.initialized = true;
        info!("WasmtimeHost initialized");

        Ok(())
    }

    fn step(&mut self, inputs: &ProcessData) -> PlcResult<ProcessData> {
        if !self.initialized {
            return Err(PlcError::Fault("Engine not initialized".into()));
        }

        let memory = self
            .memory
            .ok_or_else(|| PlcError::Fault("No memory available".into()))?;

        if self.step_fn.is_none() {
            return Err(PlcError::Fault("No step function".into()));
        }

        // Read values needed before mutable borrow
        let cycle_time_ns = self.cycle_time_ns;
        let first_cycle = self.store.data().first_cycle;

        // Copy inputs to Wasm memory
        {
            let data = memory.data_mut(&mut self.store);
            copy_inputs_to_wasm(data, inputs);

            // Update system info
            let sys_info = WasmSystemInfo {
                cycle_time_ns,
                flags: if first_cycle {
                    WasmSystemInfo::FLAG_FIRST_CYCLE
                } else {
                    0
                },
            };
            write_system_info(data, &sys_info);
        }

        // Set epoch deadline for timeout
        self.set_epoch_deadline();

        // Add fuel if fuel-based budgeting is enabled
        if self.use_fuel {
            self.store
                .set_fuel(self.fuel_per_cycle)
                .map_err(|e| PlcError::Fault(format!("Failed to set fuel: {e}")))?;
        }

        // Call step function
        if let Some(step_fn) = &self.step_fn {
            step_fn.call(&mut self.store, ()).map_err(|e| {
                // Check for out-of-fuel trap using Wasmtime's Trap enum
                if let Some(trap) = e.downcast_ref::<Trap>() {
                    if matches!(trap, Trap::OutOfFuel) {
                        return PlcError::CycleOverrun {
                            expected_ns: self.cycle_time_ns,
                            actual_ns: 0, // Unknown - fuel exhausted before completion
                        };
                    }
                }
                PlcError::WasmTrap(format!("step() failed: {e}"))
            })?;
        }

        // Copy outputs from Wasm memory
        let mut outputs = self.process_data;
        {
            let data = memory.data(&self.store);
            copy_outputs_from_wasm(data, &mut outputs);
        }

        // Advance cycle
        self.store.data_mut().advance_cycle();

        trace!(cycle = self.store.data().cycle_count, "Step completed");

        Ok(outputs)
    }

    fn fault(&mut self) -> PlcResult<()> {
        warn!("Entering fault mode");

        // Call fault function if present
        let has_fault = self.fault_fn.is_some();
        if has_fault {
            self.set_epoch_deadline();
            // Best effort - we're already in fault mode, don't fail on fuel error
            let _ = self.ensure_fuel();
            if let Some(fault_fn) = &self.fault_fn {
                fault_fn
                    .call(&mut self.store, ())
                    .map_err(|e| PlcError::WasmTrap(format!("fault() failed: {e}")))?;
            }
        }

        // Zero all outputs in memory
        let cycle_time_ns = self.cycle_time_ns;
        if let Some(memory) = self.memory {
            let data = memory.data_mut(&mut self.store);
            // Zero digital outputs (offset 4, size 4)
            if data.len() >= 8 {
                data[4..8].fill(0);
            }
            // Zero analog outputs (offset 0x28, size 32)
            if data.len() >= 0x48 {
                data[0x28..0x48].fill(0);
            }

            // Set fault flag in system info
            let sys_info = WasmSystemInfo {
                cycle_time_ns,
                flags: WasmSystemInfo::FLAG_FAULT_MODE,
            };
            write_system_info(data, &sys_info);
        }

        Ok(())
    }

    fn is_ready(&self) -> bool {
        self.initialized && self.instance.is_some()
    }

    fn start_epoch_ticker(&self) -> Option<EpochTicker> {
        // Clone the engine (internally Arc-based) for the ticker thread
        let engine = self.engine.clone();
        let epoch_counter = Arc::clone(&self.epoch_counter);

        // Tick interval: roughly 10µs per epoch as estimated in constructor
        // This allows fine-grained timeout control
        let tick_interval = Duration::from_micros(10);

        info!(
            tick_interval_us = tick_interval.as_micros(),
            "Starting epoch ticker"
        );

        let tick_cb = move || {
            engine.increment_epoch();
            epoch_counter.fetch_add(1, Ordering::Relaxed);
        };

        match EpochTicker::new(tick_interval, tick_cb) {
            Ok(ticker) => Some(ticker),
            Err(e) => {
                warn!("Failed to start epoch ticker: {}", e);
                None
            }
        }
    }

    fn reload_module(&mut self, wasm_bytes: &[u8], preserve_memory: bool) -> PlcResult<()> {
        info!(
            preserve_memory,
            bytes_len = wasm_bytes.len(),
            "Hot-reloading Wasm module"
        );

        // Compile the new module first (before touching current state)
        let new_module = Module::new(&self.engine, wasm_bytes)
            .map_err(|e| PlcError::Config(format!("Failed to compile new module: {e}")))?;

        // Verify required exports exist
        let has_step = new_module.exports().any(|e| e.name() == "step");
        let has_memory = new_module.exports().any(|e| e.name() == "memory");

        if !has_step {
            return Err(PlcError::Config(
                "New module missing required 'step' export".into(),
            ));
        }
        if !has_memory {
            return Err(PlcError::Config(
                "New module missing required 'memory' export".into(),
            ));
        }

        // Save old memory contents if preserving state
        let old_memory_data = if preserve_memory {
            self.memory.map(|mem| {
                let data = mem.data(&self.store);
                data.to_vec()
            })
        } else {
            None
        };

        // Save cycle count to preserve continuity
        let saved_cycle_count = self.store.data().cycle_count;

        // Clear old instance state
        self.instance = None;
        self.memory = None;
        self.step_fn = None;
        self.init_fn = None;
        self.fault_fn = None;

        // Set the new module
        self.module = Some(new_module);

        // Instantiate the new module
        self.instantiate()
            .map_err(|e| PlcError::Config(format!("Failed to instantiate new module: {e}")))?;

        // Restore memory contents if preserving state
        if let (Some(old_data), Some(new_memory)) = (old_memory_data, self.memory) {
            let new_data = new_memory.data_mut(&mut self.store);
            let copy_len = old_data.len().min(new_data.len());

            if copy_len > 0 {
                new_data[..copy_len].copy_from_slice(&old_data[..copy_len]);
                debug!(
                    copied_bytes = copy_len,
                    old_size = old_data.len(),
                    new_size = new_data.len(),
                    "Memory state migrated"
                );
            }

            // If new memory is smaller, warn about potential data loss
            if new_data.len() < old_data.len() {
                warn!(
                    old_size = old_data.len(),
                    new_size = new_data.len(),
                    "New module has smaller memory - some state may be lost"
                );
            }
        } else {
            // Call init on new module (either couldn't preserve memory or not requested)
            let has_init = self.init_fn.is_some();
            if has_init {
                self.set_epoch_deadline();
                self.ensure_fuel()?;
                if let Some(init_fn) = &self.init_fn {
                    init_fn.call(&mut self.store, ()).map_err(|e| {
                        PlcError::WasmTrap(format!("init() failed after reload: {e}"))
                    })?;
                }
            }
        }

        // Restore cycle count
        self.store.data_mut().cycle_count = saved_cycle_count;
        self.store.data_mut().first_cycle = false;

        info!(cycle_count = saved_cycle_count, "Hot-reload complete");

        Ok(())
    }

    fn supports_hot_reload(&self) -> bool {
        true
    }
}

/// Configuration options for WasmtimeHost.
#[derive(Debug, Clone)]
pub struct WasmtimeConfig {
    /// Cranelift optimization level.
    pub opt_level: OptLevel,
    /// Maximum linear memory size in bytes.
    pub max_memory_bytes: usize,
    /// Maximum table elements.
    pub max_table_elements: u32,
    /// Enable SIMD instructions.
    pub enable_simd: bool,
    /// Enable deterministic execution mode.
    pub deterministic: bool,
    /// Enable fuel-based execution budgeting.
    pub use_fuel: bool,
    /// Fuel units to grant per cycle.
    pub fuel_per_cycle: u64,
}

impl Default for WasmtimeConfig {
    fn default() -> Self {
        Self {
            opt_level: OptLevel::Speed,
            max_memory_bytes: 16 * 1024 * 1024, // 16 MB
            max_table_elements: 10_000,
            enable_simd: false,
            deterministic: false,
            use_fuel: false,
            fuel_per_cycle: 1_000_000,
        }
    }
}

/// A no-op logic engine for testing without Wasm.
#[derive(Debug, Default)]
pub struct NullEngine {
    initialized: bool,
}

impl LogicEngine for NullEngine {
    fn init(&mut self) -> PlcResult<()> {
        self.initialized = true;
        Ok(())
    }

    fn step(&mut self, inputs: &ProcessData) -> PlcResult<ProcessData> {
        // Pass-through: inputs become outputs
        Ok(*inputs)
    }

    fn fault(&mut self) -> PlcResult<()> {
        Ok(())
    }

    fn is_ready(&self) -> bool {
        self.initialized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLINK_WAT: &str = r#"
        (module
            (import "plc" "read_di" (func $read_di (param i32) (result i32)))
            (import "plc" "write_do" (func $write_do (param i32 i32)))
            (import "plc" "get_cycle_time" (func $get_cycle_time (result i32)))
            (import "plc" "is_first_cycle" (func $is_first_cycle (result i32)))

            (memory (export "memory") 1)

            ;; Counter at memory offset 0x100
            (global $counter (mut i32) (i32.const 0))

            (func (export "init")
                ;; Initialize counter to 0
                (global.set $counter (i32.const 0))
            )

            (func (export "step")
                ;; Increment counter
                (global.set $counter
                    (i32.add (global.get $counter) (i32.const 1))
                )

                ;; Toggle DO 0 every 500 cycles (simple blink)
                (if (i32.eq
                        (i32.rem_u (global.get $counter) (i32.const 1000))
                        (i32.const 500)
                    )
                    (then
                        ;; Read current DO state and invert
                        (call $write_do
                            (i32.const 0)
                            (i32.xor
                                (call $read_di (i32.const 0))
                                (i32.const 1)
                            )
                        )
                    )
                )
            )

            (func (export "fault")
                ;; Set all outputs to 0
                (call $write_do (i32.const 0) (i32.const 0))
            )
        )
    "#;

    const PASSTHROUGH_WAT: &str = r#"
        (module
            (import "plc" "read_di" (func $read_di (param i32) (result i32)))
            (import "plc" "write_do" (func $write_do (param i32 i32)))

            (memory (export "memory") 1)

            (func (export "step")
                ;; Pass DI 0 to DO 0
                (call $write_do
                    (i32.const 0)
                    (call $read_di (i32.const 0))
                )
            )
        )
    "#;

    #[test]
    fn test_wasmtime_host_creation() {
        let host = WasmtimeHost::new(Duration::from_millis(1)).unwrap();
        assert!(!host.is_ready());
        assert!(!host.initialized);
    }

    #[test]
    fn test_load_wat_module() {
        let mut host = WasmtimeHost::new(Duration::from_millis(1)).unwrap();
        host.load_wat(BLINK_WAT).unwrap();
        assert!(host.module.is_some());
        assert!(!host.is_ready()); // Not initialized yet
    }

    #[test]
    fn test_init_and_step() {
        let mut host = WasmtimeHost::new(Duration::from_millis(1)).unwrap();
        host.load_wat(PASSTHROUGH_WAT).unwrap();
        host.init().unwrap();
        assert!(host.is_ready());

        // Create input with DI bit 0 set
        let mut inputs = ProcessData::default();
        inputs.digital_inputs[0] = 0x01;

        // Step and check output
        let outputs = host.step(&inputs).unwrap();
        assert_eq!(outputs.digital_outputs[0] & 1, 1);

        // Clear input and verify output clears
        inputs.digital_inputs[0] = 0x00;
        let outputs = host.step(&inputs).unwrap();
        assert_eq!(outputs.digital_outputs[0] & 1, 0);
    }

    #[test]
    fn test_blink_program() {
        let mut host = WasmtimeHost::new(Duration::from_millis(1)).unwrap();
        host.load_wat(BLINK_WAT).unwrap();
        host.init().unwrap();

        let inputs = ProcessData::default();

        // Run several cycles
        for _ in 0..1000 {
            let _ = host.step(&inputs).unwrap();
        }

        // The blink program should have toggled the output
        assert!(host.store.data().cycle_count >= 1000);
    }

    #[test]
    fn test_fault_handling() {
        let mut host = WasmtimeHost::new(Duration::from_millis(1)).unwrap();
        host.load_wat(BLINK_WAT).unwrap();
        host.init().unwrap();

        // Run some cycles with output set
        let mut inputs = ProcessData::default();
        inputs.digital_inputs[0] = 1;
        for _ in 0..10 {
            let _ = host.step(&inputs).unwrap();
        }

        // Trigger fault
        host.fault().unwrap();

        // Verify outputs are zeroed
        let memory = host.memory.unwrap();
        let data = memory.data(&host.store);
        let do_value = u32::from_le_bytes(data[4..8].try_into().unwrap());
        assert_eq!(do_value, 0);
    }

    #[test]
    fn test_null_engine() {
        let mut engine = NullEngine::default();
        assert!(!engine.is_ready());

        engine.init().unwrap();
        assert!(engine.is_ready());

        let inputs = ProcessData::default();
        let outputs = engine.step(&inputs).unwrap();

        // Null engine passes through inputs
        assert_eq!(outputs.digital_inputs, inputs.digital_inputs);
    }

    #[test]
    fn test_cycle_count_advances() {
        let mut host = WasmtimeHost::new(Duration::from_millis(1)).unwrap();
        host.load_wat(PASSTHROUGH_WAT).unwrap();
        host.init().unwrap();

        let inputs = ProcessData::default();

        assert_eq!(host.store.data().cycle_count, 0);
        assert!(host.store.data().first_cycle);

        let _ = host.step(&inputs).unwrap();
        assert_eq!(host.store.data().cycle_count, 1);
        assert!(!host.store.data().first_cycle);

        let _ = host.step(&inputs).unwrap();
        assert_eq!(host.store.data().cycle_count, 2);
    }

    #[test]
    fn test_supports_hot_reload() {
        let host = WasmtimeHost::new(Duration::from_millis(1)).unwrap();
        assert!(host.supports_hot_reload());

        let null_engine = NullEngine::default();
        assert!(!null_engine.supports_hot_reload());
    }

    #[test]
    fn test_hot_reload_basic() {
        let mut host = WasmtimeHost::new(Duration::from_millis(1)).unwrap();
        host.load_wat(PASSTHROUGH_WAT).unwrap();
        host.init().unwrap();

        let inputs = ProcessData::default();

        // Run a few cycles
        for _ in 0..5 {
            let _ = host.step(&inputs).unwrap();
        }
        assert_eq!(host.store.data().cycle_count, 5);

        // Hot-reload with the same module (without preserving memory)
        let wasm_bytes = wat::parse_str(PASSTHROUGH_WAT).unwrap();
        host.reload_module(&wasm_bytes, false).unwrap();

        // Cycle count should be preserved
        assert_eq!(host.store.data().cycle_count, 5);

        // Module should still work
        let outputs = host.step(&inputs).unwrap();
        assert_eq!(host.store.data().cycle_count, 6);
        assert_eq!(outputs.digital_outputs[0] & 1, 0); // No input set
    }

    #[test]
    fn test_hot_reload_with_memory_preservation() {
        // A module that writes to memory location 0x100
        const WRITER_WAT: &str = r#"
            (module
                (import "plc" "read_di" (func $read_di (param i32) (result i32)))
                (import "plc" "write_do" (func $write_do (param i32 i32)))

                (memory (export "memory") 1)

                (func (export "init")
                    ;; Write a marker value to memory offset 0x100
                    (i32.store (i32.const 0x100) (i32.const 0xDEADBEEF))
                )

                (func (export "step")
                    ;; Read marker and output it as digital output if set
                    (call $write_do
                        (i32.const 0)
                        (i32.ne
                            (i32.load (i32.const 0x100))
                            (i32.const 0)
                        )
                    )
                )
            )
        "#;

        let mut host = WasmtimeHost::new(Duration::from_millis(1)).unwrap();
        host.load_wat(WRITER_WAT).unwrap();
        host.init().unwrap();

        // Verify marker is set in memory
        let memory = host.memory.unwrap();
        let data = memory.data(&host.store);
        let marker = u32::from_le_bytes(data[0x100..0x104].try_into().unwrap());
        assert_eq!(marker, 0xDEADBEEF);

        // Run a step to verify output is set based on marker
        let inputs = ProcessData::default();
        let outputs = host.step(&inputs).unwrap();
        assert_eq!(outputs.digital_outputs[0] & 1, 1); // Marker != 0

        // Hot-reload with memory preservation
        let wasm_bytes = wat::parse_str(WRITER_WAT).unwrap();
        host.reload_module(&wasm_bytes, true).unwrap();

        // Verify marker is still in memory after reload
        let memory = host.memory.unwrap();
        let data = memory.data(&host.store);
        let marker = u32::from_le_bytes(data[0x100..0x104].try_into().unwrap());
        assert_eq!(marker, 0xDEADBEEF);

        // Module should still work with preserved state
        let outputs = host.step(&inputs).unwrap();
        assert_eq!(outputs.digital_outputs[0] & 1, 1); // Marker still != 0
    }

    #[test]
    fn test_hot_reload_rejects_invalid_module() {
        let mut host = WasmtimeHost::new(Duration::from_millis(1)).unwrap();
        host.load_wat(PASSTHROUGH_WAT).unwrap();
        host.init().unwrap();

        // Try to reload with a module missing 'step'
        const NO_STEP_WAT: &str = r#"
            (module
                (memory (export "memory") 1)
                (func (export "init"))
            )
        "#;

        let wasm_bytes = wat::parse_str(NO_STEP_WAT).unwrap();
        let result = host.reload_module(&wasm_bytes, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("step"));

        // Original module should still work
        let inputs = ProcessData::default();
        let _ = host.step(&inputs).unwrap();
    }

    #[test]
    fn test_hot_reload_rejects_missing_memory() {
        let mut host = WasmtimeHost::new(Duration::from_millis(1)).unwrap();
        host.load_wat(PASSTHROUGH_WAT).unwrap();
        host.init().unwrap();

        // Try to reload with a module missing 'memory'
        const NO_MEMORY_WAT: &str = r#"
            (module
                (func (export "step"))
            )
        "#;

        let wasm_bytes = wat::parse_str(NO_MEMORY_WAT).unwrap();
        let result = host.reload_module(&wasm_bytes, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("memory"));
    }
}
