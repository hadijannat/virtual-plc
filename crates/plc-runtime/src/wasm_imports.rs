//! Host functions exported to Wasm PLC programs.
//!
//! These functions are imported by Wasm modules to access I/O and system services.
//! They provide a safe interface between the sandboxed Wasm code and the host runtime.
//!
//! # Imported Functions
//!
//! Wasm modules should import these functions from the "plc" module:
//!
//! ```wat
//! (import "plc" "read_di" (func $read_di (param i32) (result i32)))
//! (import "plc" "write_do" (func $write_do (param i32 i32)))
//! (import "plc" "read_ai" (func $read_ai (param i32) (result i32)))
//! (import "plc" "write_ao" (func $write_ao (param i32 i32)))
//! (import "plc" "get_cycle_time" (func $get_cycle_time (result i32)))
//! (import "plc" "log_message" (func $log_message (param i32 i32)))
//! ```

use crate::wasm_memory::{
    read_ai_from_memory, read_di_from_memory, write_ao_to_memory, write_do_to_memory,
};
use tracing::{trace, warn};
use wasmtime::{Caller, Linker, Memory};

/// Host state accessible from Wasm host functions.
#[derive(Debug)]
pub struct HostState {
    /// Reference to Wasm linear memory (set after instantiation).
    pub memory: Option<Memory>,
    /// Cycle time in nanoseconds (u64 to prevent overflow for cycles > 4.29s).
    pub cycle_time_ns: u64,
    /// Current cycle number.
    pub cycle_count: u64,
    /// Whether we're in first-cycle mode.
    pub first_cycle: bool,
    /// Log buffer for messages from Wasm.
    pub log_buffer: Vec<String>,
}

impl Default for HostState {
    fn default() -> Self {
        Self {
            memory: None,
            cycle_time_ns: 1_000_000, // 1ms default
            cycle_count: 0,
            first_cycle: true,
            log_buffer: Vec::new(),
        }
    }
}

impl HostState {
    /// Create a new host state with the given cycle time.
    pub fn new(cycle_time_ns: u64) -> Self {
        Self {
            cycle_time_ns,
            ..Default::default()
        }
    }

    /// Set the memory reference after module instantiation.
    pub fn set_memory(&mut self, memory: Memory) {
        self.memory = Some(memory);
    }

    /// Increment cycle count and clear first-cycle flag.
    pub fn advance_cycle(&mut self) {
        self.cycle_count += 1;
        self.first_cycle = false;
    }
}

/// Helper to get memory from caller, either from export or stored reference.
fn get_memory(caller: &mut Caller<'_, HostState>) -> Option<Memory> {
    // First try to get from export
    if let Some(extern_) = caller.get_export("memory") {
        if let Some(memory) = extern_.into_memory() {
            return Some(memory);
        }
    }
    // Fall back to stored reference
    caller.data().memory
}

/// Read a digital input bit.
fn host_read_di(mut caller: Caller<'_, HostState>, bit: i32) -> i32 {
    if let Some(memory) = get_memory(&mut caller) {
        let data = memory.data(&caller);
        let value = read_di_from_memory(data, bit as u32);
        trace!(bit, value, "read_di");
        if value {
            1
        } else {
            0
        }
    } else {
        warn!("read_di called without memory");
        0
    }
}

/// Write a digital output bit.
fn host_write_do(mut caller: Caller<'_, HostState>, bit: i32, value: i32) {
    if let Some(memory) = get_memory(&mut caller) {
        let data = memory.data_mut(&mut caller);
        write_do_to_memory(data, bit as u32, value != 0);
        trace!(bit, value, "write_do");
    } else {
        warn!("write_do called without memory");
    }
}

/// Read an analog input channel.
fn host_read_ai(mut caller: Caller<'_, HostState>, channel: i32) -> i32 {
    if let Some(memory) = get_memory(&mut caller) {
        let data = memory.data(&caller);
        let value = read_ai_from_memory(data, channel as u32);
        trace!(channel, value, "read_ai");
        value as i32
    } else {
        warn!("read_ai called without memory");
        0
    }
}

/// Write an analog output channel.
fn host_write_ao(mut caller: Caller<'_, HostState>, channel: i32, value: i32) {
    if let Some(memory) = get_memory(&mut caller) {
        let data = memory.data_mut(&mut caller);
        // Clamp to i16 range
        let clamped = value.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        write_ao_to_memory(data, channel as u32, clamped);
        trace!(channel, value = clamped, "write_ao");
    } else {
        warn!("write_ao called without memory");
    }
}

/// Get the cycle time in nanoseconds.
///
/// Note: Returns i32 for Wasm ABI compatibility. Values > i32::MAX are capped.
fn host_get_cycle_time(caller: Caller<'_, HostState>) -> i32 {
    let cycle_time = caller.data().cycle_time_ns;
    trace!(cycle_time_ns = cycle_time, "get_cycle_time");
    // Cap at i32::MAX for ABI compatibility (cycle times > ~2.1s will be capped)
    cycle_time.min(i32::MAX as u64) as i32
}

/// Get the current cycle count.
fn host_get_cycle_count(caller: Caller<'_, HostState>) -> i64 {
    let count = caller.data().cycle_count;
    trace!(cycle_count = count, "get_cycle_count");
    count as i64
}

/// Check if this is the first cycle after initialization.
fn host_is_first_cycle(caller: Caller<'_, HostState>) -> i32 {
    let first = caller.data().first_cycle;
    trace!(first_cycle = first, "is_first_cycle");
    if first {
        1
    } else {
        0
    }
}

/// Log a message from the Wasm module.
///
/// # Safety
///
/// This function validates that `ptr` and `len` are non-negative and that
/// the memory range `[ptr, ptr+len)` is within bounds before accessing memory.
fn host_log_message(mut caller: Caller<'_, HostState>, ptr: i32, len: i32) {
    // Validate inputs from Wasm - could be malicious
    if ptr < 0 || len < 0 {
        warn!(ptr, len, "log_message called with negative values");
        return;
    }

    // Safe to convert to usize now
    let start = ptr as usize;
    let len_usize = len as usize;

    // Check for overflow before computing end
    let end = match start.checked_add(len_usize) {
        Some(e) => e,
        None => {
            warn!(ptr, len, "log_message: ptr + len overflow");
            return;
        }
    };

    let msg = if let Some(memory) = get_memory(&mut caller) {
        let data = memory.data(&caller);
        if end <= data.len() {
            std::str::from_utf8(&data[start..end])
                .ok()
                .map(String::from)
        } else {
            warn!(
                ptr,
                len,
                memory_size = data.len(),
                "log_message: out of bounds"
            );
            None
        }
    } else {
        warn!("log_message called without memory");
        None
    };

    if let Some(msg) = msg {
        tracing::info!(wasm_log = %msg, "PLC program log");
        caller.data_mut().log_buffer.push(msg);
    }
}

/// Register all PLC host functions with a Wasmtime linker.
pub fn register_host_functions(linker: &mut Linker<HostState>) -> anyhow::Result<()> {
    // I/O functions
    linker.func_wrap("plc", "read_di", host_read_di)?;
    linker.func_wrap("plc", "write_do", host_write_do)?;
    linker.func_wrap("plc", "read_ai", host_read_ai)?;
    linker.func_wrap("plc", "write_ao", host_write_ao)?;

    // System functions
    linker.func_wrap("plc", "get_cycle_time", host_get_cycle_time)?;
    linker.func_wrap("plc", "get_cycle_count", host_get_cycle_count)?;
    linker.func_wrap("plc", "is_first_cycle", host_is_first_cycle)?;

    // Logging
    linker.func_wrap("plc", "log_message", host_log_message)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasmtime::{Config, Engine, Module, Store, TypedFunc};

    fn create_test_engine() -> Engine {
        let mut config = Config::new();
        config.epoch_interruption(true);
        Engine::new(&config).unwrap()
    }

    #[test]
    fn test_host_state_default() {
        let state = HostState::default();
        assert_eq!(state.cycle_time_ns, 1_000_000);
        assert_eq!(state.cycle_count, 0);
        assert!(state.first_cycle);
        assert!(state.memory.is_none());
    }

    #[test]
    fn test_host_state_advance_cycle() {
        let mut state = HostState::new(500_000);
        assert!(state.first_cycle);
        assert_eq!(state.cycle_count, 0);

        state.advance_cycle();
        assert!(!state.first_cycle);
        assert_eq!(state.cycle_count, 1);

        state.advance_cycle();
        assert_eq!(state.cycle_count, 2);
    }

    #[test]
    fn test_register_functions() {
        let engine = create_test_engine();
        let mut linker = Linker::new(&engine);

        // Should succeed without error
        register_host_functions(&mut linker).unwrap();
    }

    #[test]
    fn test_linker_with_minimal_module() {
        let engine = create_test_engine();
        let mut linker = Linker::new(&engine);
        register_host_functions(&mut linker).unwrap();

        // Create a minimal Wasm module that imports read_di
        let wat = r#"
            (module
                (import "plc" "read_di" (func $read_di (param i32) (result i32)))
                (import "plc" "write_do" (func $write_do (param i32 i32)))
                (import "plc" "get_cycle_time" (func $get_cycle_time (result i32)))
                (memory (export "memory") 1)
                (func (export "step")
                    ;; Read DI 0 and write to DO 0
                    (call $write_do
                        (i32.const 0)
                        (call $read_di (i32.const 0))
                    )
                )
            )
        "#;

        let module = Module::new(&engine, wat).unwrap();
        let mut store = Store::new(&engine, HostState::default());
        let instance = linker.instantiate(&mut store, &module).unwrap();

        // Get and set memory
        let memory = instance.get_memory(&mut store, "memory").unwrap();
        store.data_mut().set_memory(memory);

        // Get step function
        let step: TypedFunc<(), ()> = instance.get_typed_func(&mut store, "step").unwrap();

        // Set epoch deadline to allow execution
        store.set_epoch_deadline(1000);

        // Set DI bit 0
        let mem_data = memory.data_mut(&mut store);
        mem_data[0] = 0x01; // DI bit 0 set

        // Call step
        step.call(&mut store, ()).unwrap();

        // Check DO bit 0 is now set
        let mem_data = memory.data(&store);
        assert_eq!(mem_data[4] & 0x01, 0x01);
    }
}
