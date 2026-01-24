//! Process image with double-buffering for deterministic I/O access.
//!
//! The I/O image provides atomic access to process data between the
//! fieldbus driver (writer) and the logic engine (reader). It uses:
//!
//! - Separate double-buffers for inputs (fieldbus → logic) and outputs (logic → fieldbus)
//! - Seqlock semantics for consistent snapshots without blocking
//! - Cache-line alignment to prevent false sharing
//!
//! # Threading Model
//!
//! - **Fieldbus thread**: Writes inputs via `write_inputs()`, reads outputs via `read_outputs()`
//! - **Logic thread**: Reads inputs via `read_inputs()`, writes outputs via `write_outputs()`
//!
//! # Memory Layout
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │ Input Buffers (fieldbus writes, logic reads)                     │
//! │ ┌──────────────────────┐  ┌──────────────────────┐             │
//! │ │ Front Buffer         │  │ Back Buffer          │ + seqlock   │
//! │ └──────────────────────┘  └──────────────────────┘             │
//! ├─────────────────────────────────────────────────────────────────┤
//! │ Output Buffers (logic writes, fieldbus reads)                    │
//! │ ┌──────────────────────┐  ┌──────────────────────┐             │
//! │ │ Front Buffer         │  │ Back Buffer          │ + seqlock   │
//! │ └──────────────────────┘  └──────────────────────┘             │
//! └─────────────────────────────────────────────────────────────────┘
//! ```

use crossbeam_utils::CachePadded;
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU64, Ordering};

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
    _pad: [u8; CACHE_LINE_SIZE
        - ((DI_WORDS + DO_WORDS) * 4 + (AI_CHANNELS + AO_CHANNELS) * 2) % CACHE_LINE_SIZE],
}

impl Default for ProcessData {
    fn default() -> Self {
        Self {
            digital_inputs: [0; DI_WORDS],
            digital_outputs: [0; DO_WORDS],
            analog_inputs: [0; AI_CHANNELS],
            analog_outputs: [0; AO_CHANNELS],
            _pad: [0; CACHE_LINE_SIZE
                - ((DI_WORDS + DO_WORDS) * 4 + (AI_CHANNELS + AO_CHANNELS) * 2) % CACHE_LINE_SIZE],
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

/// A seqlock-protected double buffer for one-way data transfer.
///
/// One thread writes to the back buffer, then commits to swap it.
/// Another thread reads from the front buffer with seqlock protection.
struct SeqlockBuffer {
    /// Sequence number (odd = write in progress).
    sequence: CachePadded<AtomicU64>,
    /// Buffer 0.
    buf0: CachePadded<UnsafeCell<ProcessData>>,
    /// Buffer 1.
    buf1: CachePadded<UnsafeCell<ProcessData>>,
    /// Which buffer is currently the "published" front (0 or 1).
    front_idx: CachePadded<AtomicU64>,
}

impl SeqlockBuffer {
    fn new() -> Self {
        Self {
            sequence: CachePadded::new(AtomicU64::new(0)),
            buf0: CachePadded::new(UnsafeCell::new(ProcessData::default())),
            buf1: CachePadded::new(UnsafeCell::new(ProcessData::default())),
            front_idx: CachePadded::new(AtomicU64::new(0)),
        }
    }

    /// Read data with seqlock protection.
    /// Spins if a write is in progress, ensuring a consistent snapshot.
    fn read(&self) -> ProcessData {
        loop {
            let seq1 = self.sequence.load(Ordering::Acquire);

            // If sequence is odd, a write is in progress - spin
            if seq1 & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }

            // Read from the front buffer
            let front = self.front_idx.load(Ordering::Acquire);
            // SAFETY: We check sequence before and after to ensure consistency.
            // The writer only touches the back buffer, never the front.
            let data = if front == 0 {
                unsafe { *self.buf0.get() }
            } else {
                unsafe { *self.buf1.get() }
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
    /// Returns a mutable reference to the back buffer.
    ///
    /// # Safety
    /// Only one thread should call this at a time.
    /// The seqlock protocol ensures readers see consistent data.
    #[allow(clippy::mut_from_ref)] // Interior mutability via UnsafeCell is intentional
    fn begin_write(&self) -> &mut ProcessData {
        // Increment sequence to odd (write in progress)
        self.sequence.fetch_add(1, Ordering::Release);

        // Write to the back buffer (opposite of front)
        let front = self.front_idx.load(Ordering::Acquire);
        if front == 0 {
            // SAFETY: Single writer assumed, and we hold the seqlock
            unsafe { &mut *self.buf1.get() }
        } else {
            unsafe { &mut *self.buf0.get() }
        }
    }

    /// Commit the write and swap buffers.
    fn commit(&self) {
        // Swap front buffer index
        let old_front = self.front_idx.load(Ordering::Acquire);
        self.front_idx.store(1 - old_front, Ordering::Release);

        // Increment sequence to even (write complete)
        self.sequence.fetch_add(1, Ordering::Release);
    }

    /// Write data in one atomic operation.
    fn write<F>(&self, f: F)
    where
        F: FnOnce(&mut ProcessData),
    {
        let data = self.begin_write();
        f(data);
        self.commit();
    }
}

/// Double-buffered I/O image with separate input and output paths.
///
/// This struct is designed for a concurrent threading model where:
/// - **Fieldbus thread**: Writes inputs, reads outputs
/// - **Logic thread**: Reads inputs, writes outputs
///
/// # Thread Safety
///
/// Each direction (inputs, outputs) has its own seqlock-protected double buffer.
/// This ensures that:
/// - Input writes by fieldbus don't affect output reads
/// - Output writes by logic don't affect input reads
/// - No data races or UB under concurrent access
pub struct IoImage {
    /// Input buffers: fieldbus writes, logic reads.
    inputs: SeqlockBuffer,
    /// Output buffers: logic writes, fieldbus reads.
    outputs: SeqlockBuffer,
}

impl std::fmt::Debug for IoImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IoImage")
            .field("input_seq", &self.inputs.sequence.load(Ordering::Relaxed))
            .field("output_seq", &self.outputs.sequence.load(Ordering::Relaxed))
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
            inputs: SeqlockBuffer::new(),
            outputs: SeqlockBuffer::new(),
        }
    }

    // =========================================================================
    // INPUT PATH: Fieldbus (writer) → Logic (reader)
    // =========================================================================

    /// Get a consistent snapshot of the input data.
    ///
    /// **Called by: Logic thread**
    ///
    /// Uses seqlock to ensure a consistent read even if the fieldbus
    /// is updating inputs concurrently.
    #[inline]
    pub fn read_inputs(&self) -> ProcessData {
        self.inputs.read()
    }

    /// Begin writing to the input back buffer.
    ///
    /// **Called by: Fieldbus thread**
    ///
    /// Returns a mutable reference to the back buffer. Call `commit_inputs()`
    /// when done to make the changes visible.
    #[inline]
    pub fn begin_write_inputs(&self) -> &mut ProcessData {
        self.inputs.begin_write()
    }

    /// Commit input writes and swap buffers.
    ///
    /// **Called by: Fieldbus thread**
    #[inline]
    pub fn commit_inputs(&self) {
        self.inputs.commit();
    }

    /// Write inputs in one atomic operation.
    ///
    /// **Called by: Fieldbus thread**
    ///
    /// Convenience method that combines begin_write_inputs/commit_inputs.
    #[inline]
    pub fn write_inputs<F>(&self, f: F)
    where
        F: FnOnce(&mut ProcessData),
    {
        self.inputs.write(f);
    }

    // =========================================================================
    // OUTPUT PATH: Logic (writer) → Fieldbus (reader)
    // =========================================================================

    /// Read current output values.
    ///
    /// **Called by: Fieldbus thread**
    ///
    /// Uses seqlock to ensure a consistent read even if the logic
    /// thread is updating outputs concurrently.
    #[inline]
    pub fn read_outputs(&self) -> ProcessData {
        self.outputs.read()
    }

    /// Begin writing to the output back buffer.
    ///
    /// **Called by: Logic thread**
    ///
    /// Returns a mutable reference to the back buffer. Call `commit_outputs()`
    /// when done to make the changes visible.
    #[inline]
    pub fn begin_write_outputs(&self) -> &mut ProcessData {
        self.outputs.begin_write()
    }

    /// Commit output writes and swap buffers.
    ///
    /// **Called by: Logic thread**
    #[inline]
    pub fn commit_outputs(&self) {
        self.outputs.commit();
    }

    /// Write outputs in one atomic operation.
    ///
    /// **Called by: Logic thread**
    ///
    /// Convenience method that combines begin_write_outputs/commit_outputs.
    #[inline]
    pub fn write_outputs<F>(&self, f: F)
    where
        F: FnOnce(&mut ProcessData),
    {
        self.outputs.write(f);
    }

    /// Get mutable access to outputs (legacy API).
    ///
    /// **Called by: Logic thread (single-threaded context only)**
    ///
    /// This method requires &mut self, so it cannot be used concurrently.
    /// Prefer `write_outputs()` for the concurrent threading model.
    #[inline]
    #[deprecated(
        since = "0.1.1",
        note = "Use write_outputs() for thread-safe atomic updates via seqlock"
    )]
    pub fn outputs_mut(&mut self) -> &mut ProcessData {
        // Since we have &mut self, no concurrent access is possible.
        // Write directly to the front buffer for immediate visibility.
        let front = self.outputs.front_idx.load(Ordering::Acquire);
        if front == 0 {
            unsafe { &mut *self.outputs.buf0.get() }
        } else {
            unsafe { &mut *self.outputs.buf1.get() }
        }
    }

    // =========================================================================
    // CONVENIENCE METHODS
    // =========================================================================

    /// Read digital inputs word (0-31).
    #[inline]
    pub fn read_di(&self) -> u32 {
        self.read_inputs().digital_inputs[0]
    }

    /// Write digital outputs word (0-31).
    #[inline]
    #[deprecated(
        since = "0.1.1",
        note = "Use write_outputs() for thread-safe atomic updates via seqlock"
    )]
    #[allow(deprecated)]
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
    #[deprecated(
        since = "0.1.1",
        note = "Use write_outputs() for thread-safe atomic updates via seqlock"
    )]
    #[allow(deprecated)]
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
    #[deprecated(
        since = "0.1.1",
        note = "Use write_outputs() for thread-safe atomic updates via seqlock"
    )]
    #[allow(deprecated)]
    pub fn write_ao(&mut self, channel: usize, value: i16) {
        self.outputs_mut().write_ao(channel, value);
    }
}

// SAFETY: IoImage is safe to send between threads.
// Each SeqlockBuffer uses atomic operations for synchronization.
// The seqlock protocol ensures readers always get consistent data.
// Writers are assumed to be single-threaded per buffer (fieldbus for inputs,
// logic for outputs), which is enforced by the API design.
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

        // Write inputs (simulating fieldbus)
        io.write_inputs(|data| {
            data.digital_inputs[0] = 0xFF;
            data.analog_inputs[0] = 100;
        });

        // Read inputs (simulating logic engine)
        let inputs = io.read_inputs();
        assert_eq!(inputs.digital_inputs[0], 0xFF);
        assert_eq!(inputs.analog_inputs[0], 100);

        // Write outputs (simulating logic engine)
        io.write_do(0xAA);
        io.write_ao(0, 500);

        // Read outputs (simulating fieldbus)
        let outputs = io.read_outputs();
        assert_eq!(outputs.digital_outputs[0], 0xAA);
        assert_eq!(outputs.analog_outputs[0], 500);
    }

    #[test]
    fn test_io_image_double_buffer_inputs() {
        let io = IoImage::new();

        // First input write
        io.write_inputs(|data| {
            data.digital_inputs[0] = 1;
        });

        let read1 = io.read_inputs();
        assert_eq!(read1.digital_inputs[0], 1);

        // Second input write should go to other buffer
        io.write_inputs(|data| {
            data.digital_inputs[0] = 2;
        });

        let read2 = io.read_inputs();
        assert_eq!(read2.digital_inputs[0], 2);
    }

    #[test]
    fn test_io_image_double_buffer_outputs() {
        let io = IoImage::new();

        // First output write
        io.write_outputs(|data| {
            data.digital_outputs[0] = 0xAA;
        });

        let read1 = io.read_outputs();
        assert_eq!(read1.digital_outputs[0], 0xAA);

        // Second output write
        io.write_outputs(|data| {
            data.digital_outputs[0] = 0xBB;
        });

        let read2 = io.read_outputs();
        assert_eq!(read2.digital_outputs[0], 0xBB);
    }

    #[test]
    fn test_input_output_isolation() {
        let io = IoImage::new();

        // Write to inputs
        io.write_inputs(|data| {
            data.digital_inputs[0] = 0xFF;
            data.digital_outputs[0] = 0x11; // This should not affect output buffer
        });

        // Write to outputs
        io.write_outputs(|data| {
            data.digital_outputs[0] = 0xAA;
            data.digital_inputs[0] = 0x22; // This should not affect input buffer
        });

        // Verify isolation
        let inputs = io.read_inputs();
        assert_eq!(inputs.digital_inputs[0], 0xFF);
        // Input buffer's digital_outputs field is not the same as output buffer

        let outputs = io.read_outputs();
        assert_eq!(outputs.digital_outputs[0], 0xAA);
        // Output buffer's digital_inputs field is not the same as input buffer
    }

    #[test]
    fn test_sequence_numbers() {
        let io = IoImage::new();

        // Initial input sequence should be 0 (even)
        assert_eq!(io.inputs.sequence.load(Ordering::Relaxed), 0);
        // Initial output sequence should be 0 (even)
        assert_eq!(io.outputs.sequence.load(Ordering::Relaxed), 0);

        // After an input write, input sequence should be 2 (even)
        io.write_inputs(|_| {});
        assert_eq!(io.inputs.sequence.load(Ordering::Relaxed), 2);
        assert_eq!(io.outputs.sequence.load(Ordering::Relaxed), 0); // Unchanged

        // After an output write, output sequence should be 2 (even)
        io.write_outputs(|_| {});
        assert_eq!(io.inputs.sequence.load(Ordering::Relaxed), 2); // Unchanged
        assert_eq!(io.outputs.sequence.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn test_concurrent_read_write() {
        use std::sync::Arc;
        use std::thread;

        let io = Arc::new(IoImage::new());
        let io_writer = Arc::clone(&io);
        let io_reader = Arc::clone(&io);

        // Writer thread continuously writes incrementing values
        let writer = thread::spawn(move || {
            for i in 0..1000u32 {
                io_writer.write_inputs(|data| {
                    data.digital_inputs[0] = i;
                });
            }
        });

        // Reader thread continuously reads
        let reader = thread::spawn(move || {
            let mut last_seen = 0u32;
            for _ in 0..1000 {
                let data = io_reader.read_inputs();
                // Values should be monotonically increasing (or same)
                // and should never be torn (partial writes)
                assert!(
                    data.digital_inputs[0] >= last_seen,
                    "Value went backwards: {} -> {}",
                    last_seen,
                    data.digital_inputs[0]
                );
                last_seen = data.digital_inputs[0];
            }
        });

        writer.join().unwrap();
        reader.join().unwrap();
    }
}
