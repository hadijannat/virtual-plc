//! Wasm linear memory layout for process image access.
//!
//! This module defines how the PLC process image is mapped into
//! WebAssembly linear memory, allowing Wasm modules to access I/O.
//!
//! # Memory Layout
//!
//! The Wasm module's linear memory is organized as:
//!
//! ```text
//! Offset    Size    Description
//! ──────────────────────────────────────
//! 0x0000    4       Digital inputs (32 bits)
//! 0x0004    4       Digital outputs (32 bits)
//! 0x0008    32      Analog inputs (16 × i16)
//! 0x0028    32      Analog outputs (16 × i16)
//! 0x0048    8       System info (cycle time, flags)
//! 0x0050    ...     User data area
//! ```
//!
//! The host runtime copies I/O data into these fixed offsets before
//! calling the Wasm step() function, and reads outputs after.

use crate::io_image::ProcessData;

/// Base offset for digital inputs in Wasm memory.
pub const WASM_DI_OFFSET: u32 = 0x0000;
/// Base offset for digital outputs in Wasm memory.
pub const WASM_DO_OFFSET: u32 = 0x0004;
/// Base offset for analog inputs in Wasm memory.
pub const WASM_AI_OFFSET: u32 = 0x0008;
/// Base offset for analog outputs in Wasm memory.
pub const WASM_AO_OFFSET: u32 = 0x0028;
/// Base offset for system info in Wasm memory.
pub const WASM_SYSINFO_OFFSET: u32 = 0x0048;
/// Start of user data area.
pub const WASM_USER_DATA_OFFSET: u32 = 0x0050;

/// Size of the I/O region in bytes.
pub const WASM_IO_REGION_SIZE: u32 = WASM_USER_DATA_OFFSET;

/// Number of digital input words.
pub const DI_WORDS: usize = 1;
/// Number of digital output words.
pub const DO_WORDS: usize = 1;
/// Number of analog input channels.
pub const AI_CHANNELS: usize = 16;
/// Number of analog output channels.
pub const AO_CHANNELS: usize = 16;

/// System info structure in Wasm memory.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct WasmSystemInfo {
    /// Cycle time in nanoseconds.
    pub cycle_time_ns: u32,
    /// Runtime flags (bit 0 = first cycle, bit 1 = fault mode).
    pub flags: u32,
}

impl WasmSystemInfo {
    /// Flag indicating this is the first cycle after init.
    pub const FLAG_FIRST_CYCLE: u32 = 0x01;
    /// Flag indicating the runtime is in fault mode.
    pub const FLAG_FAULT_MODE: u32 = 0x02;
}

/// Copy process data into Wasm linear memory.
///
/// # Safety
///
/// The caller must ensure that:
/// - `memory` points to valid Wasm linear memory
/// - `memory` has at least `WASM_IO_REGION_SIZE` bytes available
#[inline]
pub fn copy_inputs_to_wasm(memory: &mut [u8], data: &ProcessData) {
    // Digital inputs
    let di_offset = WASM_DI_OFFSET as usize;
    if memory.len() >= di_offset + 4 {
        memory[di_offset..di_offset + 4].copy_from_slice(&data.digital_inputs[0].to_le_bytes());
    }

    // Analog inputs (16 × i16 = 32 bytes)
    let ai_offset = WASM_AI_OFFSET as usize;
    for (i, &value) in data.analog_inputs.iter().enumerate() {
        let offset = ai_offset + i * 2;
        if memory.len() >= offset + 2 {
            memory[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        }
    }
}

/// Copy process outputs from Wasm linear memory.
///
/// # Safety
///
/// The caller must ensure that:
/// - `memory` points to valid Wasm linear memory
/// - `memory` has at least `WASM_IO_REGION_SIZE` bytes available
#[inline]
pub fn copy_outputs_from_wasm(memory: &[u8], data: &mut ProcessData) {
    // Digital outputs
    let do_offset = WASM_DO_OFFSET as usize;
    if memory.len() >= do_offset + 4 {
        data.digital_outputs[0] =
            u32::from_le_bytes(memory[do_offset..do_offset + 4].try_into().unwrap_or([0; 4]));
    }

    // Analog outputs (16 × i16 = 32 bytes)
    let ao_offset = WASM_AO_OFFSET as usize;
    for (i, value) in data.analog_outputs.iter_mut().enumerate() {
        let offset = ao_offset + i * 2;
        if memory.len() >= offset + 2 {
            *value =
                i16::from_le_bytes(memory[offset..offset + 2].try_into().unwrap_or([0; 2]));
        }
    }
}

/// Write system info to Wasm linear memory.
#[inline]
pub fn write_system_info(memory: &mut [u8], info: &WasmSystemInfo) {
    let offset = WASM_SYSINFO_OFFSET as usize;
    if memory.len() >= offset + 8 {
        memory[offset..offset + 4].copy_from_slice(&info.cycle_time_ns.to_le_bytes());
        memory[offset + 4..offset + 8].copy_from_slice(&info.flags.to_le_bytes());
    }
}

/// Read a digital input bit from memory.
#[inline]
pub fn read_di_from_memory(memory: &[u8], bit: u32) -> bool {
    let offset = WASM_DI_OFFSET as usize;
    if bit >= 32 || memory.len() < offset + 4 {
        return false;
    }
    let word = u32::from_le_bytes(memory[offset..offset + 4].try_into().unwrap_or([0; 4]));
    (word >> bit) & 1 != 0
}

/// Write a digital output bit to memory.
#[inline]
pub fn write_do_to_memory(memory: &mut [u8], bit: u32, value: bool) {
    let offset = WASM_DO_OFFSET as usize;
    if bit >= 32 || memory.len() < offset + 4 {
        return;
    }
    let mut word = u32::from_le_bytes(memory[offset..offset + 4].try_into().unwrap_or([0; 4]));
    if value {
        word |= 1 << bit;
    } else {
        word &= !(1 << bit);
    }
    memory[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
}

/// Read an analog input from memory.
#[inline]
pub fn read_ai_from_memory(memory: &[u8], channel: u32) -> i16 {
    if channel >= AI_CHANNELS as u32 {
        return 0;
    }
    let offset = WASM_AI_OFFSET as usize + (channel as usize) * 2;
    if memory.len() < offset + 2 {
        return 0;
    }
    i16::from_le_bytes(memory[offset..offset + 2].try_into().unwrap_or([0; 2]))
}

/// Write an analog output to memory.
#[inline]
pub fn write_ao_to_memory(memory: &mut [u8], channel: u32, value: i16) {
    if channel >= AO_CHANNELS as u32 {
        return;
    }
    let offset = WASM_AO_OFFSET as usize + (channel as usize) * 2;
    if memory.len() < offset + 2 {
        return;
    }
    memory[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

/// Get the cycle time from system info.
#[inline]
pub fn read_cycle_time_from_memory(memory: &[u8]) -> u32 {
    let offset = WASM_SYSINFO_OFFSET as usize;
    if memory.len() < offset + 4 {
        return 0;
    }
    u32::from_le_bytes(memory[offset..offset + 4].try_into().unwrap_or([0; 4]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_layout_constants() {
        // Verify offsets don't overlap
        assert!(WASM_DI_OFFSET + 4 <= WASM_DO_OFFSET);
        assert!(WASM_DO_OFFSET + 4 <= WASM_AI_OFFSET);
        assert!(WASM_AI_OFFSET + 32 <= WASM_AO_OFFSET);
        assert!(WASM_AO_OFFSET + 32 <= WASM_SYSINFO_OFFSET);
        assert!(WASM_SYSINFO_OFFSET + 8 <= WASM_USER_DATA_OFFSET);
    }

    #[test]
    fn test_copy_inputs() {
        let mut memory = vec![0u8; 256];
        let mut data = ProcessData::default();
        data.digital_inputs[0] = 0xDEAD_BEEF;
        data.analog_inputs[0] = 1000;
        data.analog_inputs[15] = -500;

        copy_inputs_to_wasm(&mut memory, &data);

        // Check digital inputs
        let di = u32::from_le_bytes(memory[0..4].try_into().unwrap());
        assert_eq!(di, 0xDEAD_BEEF);

        // Check analog inputs
        let ai0 = i16::from_le_bytes(memory[8..10].try_into().unwrap());
        assert_eq!(ai0, 1000);
        let ai15 = i16::from_le_bytes(memory[38..40].try_into().unwrap());
        assert_eq!(ai15, -500);
    }

    #[test]
    fn test_copy_outputs() {
        let mut memory = vec![0u8; 256];
        // Set digital outputs in memory
        memory[4..8].copy_from_slice(&0xCAFE_BABEu32.to_le_bytes());
        // Set analog outputs
        memory[40..42].copy_from_slice(&2000i16.to_le_bytes());
        memory[70..72].copy_from_slice(&(-1000i16).to_le_bytes());

        let mut data = ProcessData::default();
        copy_outputs_from_wasm(&memory, &mut data);

        assert_eq!(data.digital_outputs[0], 0xCAFE_BABE);
        assert_eq!(data.analog_outputs[0], 2000);
        assert_eq!(data.analog_outputs[15], -1000);
    }

    #[test]
    fn test_di_do_bit_access() {
        let mut memory = vec![0u8; 256];

        // Write digital output bits
        write_do_to_memory(&mut memory, 0, true);
        write_do_to_memory(&mut memory, 7, true);
        write_do_to_memory(&mut memory, 31, true);

        let do_word = u32::from_le_bytes(memory[4..8].try_into().unwrap());
        assert_eq!(do_word, 0x8000_0081);

        // Set digital inputs and read
        memory[0..4].copy_from_slice(&0x0000_00FFu32.to_le_bytes());
        assert!(read_di_from_memory(&memory, 0));
        assert!(read_di_from_memory(&memory, 7));
        assert!(!read_di_from_memory(&memory, 8));
    }

    #[test]
    fn test_analog_access() {
        let mut memory = vec![0u8; 256];

        // Write analog outputs
        write_ao_to_memory(&mut memory, 0, 4095);
        write_ao_to_memory(&mut memory, 5, -2048);

        // Verify in memory
        let ao0 = i16::from_le_bytes(memory[40..42].try_into().unwrap());
        let ao5 = i16::from_le_bytes(memory[50..52].try_into().unwrap());
        assert_eq!(ao0, 4095);
        assert_eq!(ao5, -2048);

        // Write analog inputs to memory and read
        memory[8..10].copy_from_slice(&1234i16.to_le_bytes());
        assert_eq!(read_ai_from_memory(&memory, 0), 1234);
    }

    #[test]
    fn test_system_info() {
        let mut memory = vec![0u8; 256];
        let info = WasmSystemInfo {
            cycle_time_ns: 1_000_000,
            flags: WasmSystemInfo::FLAG_FIRST_CYCLE,
        };

        write_system_info(&mut memory, &info);

        assert_eq!(read_cycle_time_from_memory(&memory), 1_000_000);
        let flags = u32::from_le_bytes(memory[76..80].try_into().unwrap());
        assert_eq!(flags, 1);
    }
}
