//! Fuzz target for the Structured Text parser.
//!
//! This target tests the parser with arbitrary byte sequences to find:
//! - Panics and assertion failures
//! - Stack overflows from deeply nested constructs
//! - Excessive memory allocation from pathological inputs
//!
//! # Running
//!
//! ```bash
//! cd crates/plc-compiler
//! cargo +nightly fuzz run fuzz_parse
//! ```
//!
//! # Corpus
//!
//! To seed the corpus with valid programs:
//! ```bash
//! cp examples/*.st fuzz/corpus/fuzz_parse/
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Convert to UTF-8 string, fuzzing may produce invalid UTF-8
    if let Ok(source) = std::str::from_utf8(data) {
        // Limit input size to prevent trivial resource exhaustion
        if source.len() > 100_000 {
            return;
        }

        // The parser should never panic on any input
        let _ = plc_compiler::frontend::parse(source);
    }
});
