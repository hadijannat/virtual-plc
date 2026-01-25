//! Fuzz target for the complete ST-to-Wasm compilation pipeline.
//!
//! This is the most comprehensive fuzz target, testing:
//! - Parser (frontend)
//! - Type checker
//! - IR lowering
//! - Wasm code generation
//!
//! Any crash or panic in any stage is a bug to be fixed.
//!
//! # Running
//!
//! ```bash
//! cd crates/plc-compiler
//! cargo +nightly fuzz run fuzz_compile
//! ```
//!
//! # Coverage-guided fuzzing
//!
//! The fuzzer will automatically discover interesting inputs by tracking
//! code coverage. To seed with valid programs:
//!
//! ```bash
//! mkdir -p fuzz/corpus/fuzz_compile
//! cp examples/*.st fuzz/corpus/fuzz_compile/
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(source) = std::str::from_utf8(data) {
        // Limit input size to prevent trivial DoS
        // Valid programs are typically <10KB, 100KB is generous
        if source.len() > 100_000 {
            return;
        }

        // The full compilation pipeline should never panic
        // It may return errors (expected), but should not crash
        let _ = plc_compiler::compile(source);
    }
});
