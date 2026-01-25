//! Fuzz target for lexer components.
//!
//! This target tests specific lexer functions that parse literal values,
//! which are common sources of integer overflow and parsing bugs.
//!
//! # Running
//!
//! ```bash
//! cd crates/plc-compiler
//! cargo +nightly fuzz run fuzz_lex
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        // Limit input to reasonable size for literals
        if s.len() > 1000 {
            return;
        }

        // Fuzz time literal parsing (T#..., TIME#...)
        // This handles complex duration parsing with potential overflow
        let _ = plc_compiler::frontend::lexer::parse_time_literal(s);

        // Also test with common prefixes
        for prefix in &["T#", "TIME#", "t#", "time#"] {
            let with_prefix = format!("{}{}", prefix, s);
            let _ = plc_compiler::frontend::lexer::parse_time_literal(&with_prefix);
        }
    }
});
