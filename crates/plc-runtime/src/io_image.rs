//! Process image with double-buffering for deterministic I/O access.
//!
//! The I/O image provides atomic access to process data between the
//! fieldbus driver (writer) and the logic engine (reader). It uses:
//!
//! - Double-buffering to allow concurrent read/write without locks
//! - Cache-line alignment to prevent false sharing
//! - Seqlock semantics for consistent snapshots
//!
//! # Memory Layout
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │ ProcessImage (cache-aligned)                                     │
//! │ ┌──────────────────────┐  ┌──────────────────────┐             │
//! │ │ Digital Inputs (32)  │  │ Digital Outputs (32) │             │
//! │ │ di_0..di_31          │  │ do_0..do_31          │             │
//! │ └──────────────────────┘  └──────────────────────┘             │
//! │ ┌──────────────────────┐  ┌──────────────────────┐             │
//! │ │ Analog Inputs (16)   │  │ Analog Outputs (16)  │             │
//! │ │ ai_0..ai_15 (i16)    │  │ ao_0..ao_15 (i16)    │             │
//! │ └──────────────────────┘  └──────────────────────┘             │
//! └─────────────────────────────────────────────────────────────────┘
//! ```

use crossbeam_utils::CachePadded;
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Cache line size for alignment (common x86_64 value).
const CACHE_LINE_SIZE: usize = 64;

/// Number of digital input/output words (32 bits each).
const DI_WORDS: usize = 1;
const DO_WORDS: usize = 1;

/// Number of analog input/output channels.
const AI_CHANNELS: usize = 16;
const AO_CHANNELS: usize = 16;

/// Raw process data structure, cache-line aligned.
///
/// This struct holds the actual I/O values. It's designed to fit
/// within cache lines efficiently and avoid false sharing.
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct ProcessData {
    /// Digital inputs (32 bits = 32 discrete inputs).
    pub digital_inputs: [u32; DI_WORDS],
    /// Digital outputs (32 bits = 32 discrete outputs).
    pub digital_outputs: [u32; DO_WORDS],
    /// Analog inputs (16-bit signed values).
    pub analog_inputs: [i16; AI_CHANNELS],
    /// Analog outputs (16-bit signed values).
    pub analog_outputs: [i16; AO_CHANNELS],
    /// Padding to ensure cache line alignment.
    _pad: [u8; CACHE_LINE_SIZE - ((DI_WORDS + DO_WORDS) * 4 + (AI_CHANNELS + AO_CHANNELS) * 2) % CACHE_LINE_SIZE],
}

impl Default for ProcessData {
    fn default() -> Self {
        Self {
            digital_inputs: [0; DI_WORDS],
            digital_outputs: [0; DO_WORDS],
            analog_inputs: [0; AI_CHANNELS],
            analog_outputs: [0; AO_CHANNELS],
            _pad: [0; CACHE_LINE_SIZE - ((DI_WORDS + DO_WORDS) * 4 + (AI_CHANNELS + AO_CHANNELS) * 2) % CACHE_LINE_SIZE],
        }
    }
}

impl ProcessData {
    /// Read a digital input bit.
    #[inline]
    pub fn read_di(&self, bit: usize) -> bool {
        let word = bit / 32;
        let offset = bit % 32;
        if word < DI_WORDS {
            (self.digital_inputs[word] >> offset) & 1 != 0
        } else {
            false
        }
    }

    /// Read all digital inputs as a u32.
    #[inline]
    pub fn read_di_word(&self, word: usize) -> u32 {
        self.digital_inputs.get(word).copied().unwrap_or(0)
    }

    /// Write a digital output bit.
    #[inline]
    pub fn write_do(&mut self, bit: usize, value: bool) {
        let word = bit / 32;
        let offset = bit % 32;
        if word < DO_WORDS {
            if value {
                self.digital_outputs[word] |= 1 << offset;
            } else {
                self.digital_outputs[word] &= !(1 << offset);
            }
        }
    }

    /// Write all digital outputs as a u32.
    #[inline]
    pub fn write_do_word(&mut self, word: usize, value: u32) {
        if let Some(w) = self.digital_outputs.get_mut(word) {
            *w = value;
        }
    }

    /// Read an analog input.
    #[inline]
    pub fn read_ai(&self, channel: usize) -> i16 {
        self.analog_inputs.get(channel).copied().unwrap_or(0)
    }

    /// Write an analog output.
    #[inline]
    pub fn write_ao(&mut self, channel: usize, value: i16) {
        if let Some(ao) = self.analog_outputs.get_mut(channel) {
            *ao = value;
        }
    }
}

/// Double-buffered I/O image with seqlock synchronization.
///
/// The fieldbus driver writes to inputs, the logic engine reads inputs
/// and writes outputs, then the fieldbus driver reads outputs.
///
/// # Thread Safety
///
/// - Single writer (fieldbus) for inputs
/// - Single writer (logic engine) for outputs
/// - Multiple readers allowed via seqlock
pub struct IoImage {
    /// Sequence number for seqlock (odd = write in progress).
    sequence: CachePadded<AtomicU64>,

    /// Front buffer (currently active for readers).
    front: CachePadded<UnsafeCell<ProcessData>>,

    /// Back buffer (being written by fieldbus).
    back: CachePadded<UnsafeCell<ProcessData>>,

    /// Which buffer is front (0 or 1). Atomic for safe swapping.
    active_buffer: AtomicU32,
}

impl std::fmt::Debug for IoImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IoImage")
            .field("sequence", &self.sequence.load(Ordering::Relaxed))
            .field("active_buffer", &self.active_buffer.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl Default for IoImage {
    fn default() -> Self {
        Self::new()
    }
}

impl IoImage {
    /// Create a new double-buffered I/O image.
    pub fn new() -> Self {
        Self {
            sequence: CachePadded::new(AtomicU64::new(0)),
            front: CachePadded::new(UnsafeCell::new(ProcessData::default())),
            back: CachePadded::new(UnsafeCell::new(ProcessData::default())),
            active_buffer: AtomicU32::new(0),
        }
    }

    /// Get a consistent snapshot of the input data.
    ///
    /// Uses seqlock to ensure a consistent read even if the writer
    /// is updating the back buffer concurrently.
    #[inline]
    pub fn read_inputs(&self) -> ProcessData {
        loop {
            let seq1 = self.sequence.load(Ordering::Acquire);

            // If sequence is odd, a write is in progress - spin
            if seq1 & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }

            // Read the data
            // SAFETY: We check sequence before and after to ensure consistency
            let data = if self.active_buffer.load(Ordering::Acquire) == 0 {
                unsafe { *self.front.get() }
            } else {
                unsafe { *self.back.get() }
            };

            // Verify sequence hasn't changed
            let seq2 = self.sequence.load(Ordering::Acquire);
            if seq1 == seq2 {
                return data;
            }

            // Sequence changed during read - retry
            std::hint::spin_loop();
        }
    }

    /// Begin writing to the back buffer.
    ///
    /// Returns a mutable reference to the back buffer. Call `commit_inputs()`
    /// when done to make the changes visible.
    ///
    /// # Safety
    ///
    /// Only one writer should call this at a time. The seqlock ensures
    /// readers see a consistent state.
    #[inline]
    pub fn begin_write_inputs(&self) -> &mut ProcessData {
        // Increment sequence to odd (write in progress)
        self.sequence.fetch_add(1, Ordering::Release);

        // Get mutable reference to the inactive buffer
        let active = self.active_buffer.load(Ordering::Acquire);
        if active == 0 {
            // Front is active, write to back
            // SAFETY: We hold the seqlock write, single writer assumed
            unsafe { &mut *self.back.get() }
        } else {
            // Back is active, write to front
            unsafe { &mut *self.front.get() }
        }
    }

    /// Commit input writes and swap buffers.
    ///
    /// Makes the back buffer become the new front buffer.
    #[inline]
    pub fn commit_inputs(&self) {
        // Swap active buffer
        let old = self.active_buffer.load(Ordering::Acquire);
        self.active_buffer.store(1 - old, Ordering::Release);

        // Increment sequence to even (write complete)
        self.sequence.fetch_add(1, Ordering::Release);
    }

    /// Write inputs in one atomic operation.
    ///
    /// This is a convenience method that combines begin/commit.
    #[inline]
    pub fn write_inputs<F>(&self, f: F)
    where
        F: FnOnce(&mut ProcessData),
    {
        let data = self.begin_write_inputs();
        f(data);
        self.commit_inputs();
    }

    /// Get mutable access to outputs.
    ///
    /// The logic engine uses this to set output values.
    /// No locking needed as only the logic engine writes outputs.
    #[inline]
    pub fn outputs_mut(&mut self) -> &mut ProcessData {
        let active = self.active_buffer.load(Ordering::Acquire);
        if active == 0 {
            // SAFETY: We have &mut self, so exclusive access is guaranteed
            unsafe { &mut *self.front.get() }
        } else {
            unsafe { &mut *self.back.get() }
        }
    }

    /// Read current output values.
    ///
    /// Used by the fieldbus driver to send outputs to the field.
    #[inline]
    pub fn read_outputs(&self) -> ProcessData {
        let active = self.active_buffer.load(Ordering::Acquire);
        // SAFETY: Outputs are only written by logic engine with &mut self
        if active == 0 {
            unsafe { *self.front.get() }
        } else {
            unsafe { *self.back.get() }
        }
    }

    // === Convenience methods for common access patterns ===

    /// Read a digital input bit (0-31).
    #[inline]
    pub fn read_di(&self) -> u32 {
        self.read_inputs().digital_inputs[0]
    }

    /// Write digital outputs (0-31).
    #[inline]
    pub fn write_do(&mut self, value: u32) {
        self.outputs_mut().digital_outputs[0] = value;
    }

    /// Read a single digital input bit.
    #[inline]
    pub fn read_di_bit(&self, bit: usize) -> bool {
        self.read_inputs().read_di(bit)
    }

    /// Write a single digital output bit.
    #[inline]
    pub fn write_do_bit(&mut self, bit: usize, value: bool) {
        self.outputs_mut().write_do(bit, value);
    }

    /// Read an analog input channel.
    #[inline]
    pub fn read_ai(&self, channel: usize) -> i16 {
        self.read_inputs().read_ai(channel)
    }

    /// Write an analog output channel.
    #[inline]
    pub fn write_ao(&mut self, channel: usize, value: i16) {
        self.outputs_mut().write_ao(channel, value);
    }
}

// SAFETY: IoImage uses atomic operations for all shared state
unsafe impl Send for IoImage {}
unsafe impl Sync for IoImage {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_data_alignment() {
        assert_eq!(std::mem::align_of::<ProcessData>(), CACHE_LINE_SIZE);
    }

    #[test]
    fn test_digital_io() {
        let mut data = ProcessData::default();

        // Test bit operations
        assert!(!data.read_di(0));
        data.digital_inputs[0] = 0b1010;
        assert!(!data.read_di(0));
        assert!(data.read_di(1));
        assert!(!data.read_di(2));
        assert!(data.read_di(3));

        // Test output bit operations
        data.write_do(5, true);
        assert_eq!(data.digital_outputs[0], 0b100000);
        data.write_do(5, false);
        assert_eq!(data.digital_outputs[0], 0);
    }

    #[test]
    fn test_analog_io() {
        let mut data = ProcessData::default();

        data.analog_inputs[0] = 1000;
        data.analog_inputs[15] = -1000;

        assert_eq!(data.read_ai(0), 1000);
        assert_eq!(data.read_ai(15), -1000);
        assert_eq!(data.read_ai(100), 0); // Out of bounds returns 0

        data.write_ao(0, 2000);
        assert_eq!(data.analog_outputs[0], 2000);
    }

    #[test]
    fn test_io_image_basic() {
        let mut io = IoImage::new();

        // Write inputs
        io.write_inputs(|data| {
            data.digital_inputs[0] = 0xFF;
            data.analog_inputs[0] = 100;
        });

        // Read inputs
        let inputs = io.read_inputs();
        assert_eq!(inputs.digital_inputs[0], 0xFF);
        assert_eq!(inputs.analog_inputs[0], 100);

        // Write outputs
        io.write_do(0xAA);
        io.write_ao(0, 500);

        let outputs = io.read_outputs();
        assert_eq!(outputs.digital_outputs[0], 0xAA);
        assert_eq!(outputs.analog_outputs[0], 500);
    }

    #[test]
    fn test_io_image_double_buffer() {
        let io = IoImage::new();

        // First write
        io.write_inputs(|data| {
            data.digital_inputs[0] = 1;
        });

        let read1 = io.read_inputs();
        assert_eq!(read1.digital_inputs[0], 1);

        // Second write should go to other buffer
        io.write_inputs(|data| {
            data.digital_inputs[0] = 2;
        });

        let read2 = io.read_inputs();
        assert_eq!(read2.digital_inputs[0], 2);
    }

    #[test]
    fn test_sequence_number() {
        let io = IoImage::new();

        // Initial sequence should be 0 (even)
        assert_eq!(io.sequence.load(Ordering::Relaxed), 0);

        // After a write, sequence should be 2 (even)
        io.write_inputs(|_| {});
        assert_eq!(io.sequence.load(Ordering::Relaxed), 2);

        // After another write, sequence should be 4
        io.write_inputs(|_| {});
        assert_eq!(io.sequence.load(Ordering::Relaxed), 4);
    }
}
