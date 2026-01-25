# Fuzzing the PLC Compiler

This directory contains fuzz targets for the `plc-compiler` crate using [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) and libFuzzer.

## Prerequisites

1. Install cargo-fuzz (requires nightly Rust):
   ```bash
   cargo install cargo-fuzz
   ```

2. Ensure you have nightly Rust:
   ```bash
   rustup install nightly
   ```

## Available Fuzz Targets

| Target | Description |
|--------|-------------|
| `fuzz_parse` | Tests the Structured Text parser |
| `fuzz_lex` | Tests lexer components (time literal parsing) |
| `fuzz_compile` | Tests the full ST → Wasm compilation pipeline |

## Running the Fuzzer

From the `crates/plc-compiler` directory:

```bash
# Run the parser fuzzer
cargo +nightly fuzz run fuzz_parse

# Run the full compilation fuzzer
cargo +nightly fuzz run fuzz_compile

# Run the lexer fuzzer
cargo +nightly fuzz run fuzz_lex
```

### With Limits

```bash
# Run for 5 minutes
cargo +nightly fuzz run fuzz_parse -- -max_total_time=300

# Run with 4 jobs in parallel
cargo +nightly fuzz run fuzz_parse -- -jobs=4 -workers=4

# Limit memory usage to 2GB
cargo +nightly fuzz run fuzz_parse -- -rss_limit_mb=2048
```

## Seeding the Corpus

The fuzzer works better when seeded with valid inputs:

```bash
# Seed with example ST programs
mkdir -p fuzz/corpus/fuzz_compile
cp ../../examples/*.st fuzz/corpus/fuzz_compile/ 2>/dev/null || true

# Seed parser with minimal programs
mkdir -p fuzz/corpus/fuzz_parse
echo 'PROGRAM X VAR a:INT; END_VAR a:=1; END_PROGRAM' > fuzz/corpus/fuzz_parse/minimal.st
```

## Reproducing Crashes

When the fuzzer finds a crash, it saves the input in `fuzz/artifacts/`. To reproduce:

```bash
# Reproduce a specific crash
cargo +nightly fuzz run fuzz_parse fuzz/artifacts/fuzz_parse/crash-<hash>
```

## Minimizing Crash Inputs

To get a minimal reproducer:

```bash
cargo +nightly fuzz tmin fuzz_parse fuzz/artifacts/fuzz_parse/crash-<hash>
```

## Coverage Report

To see what code paths have been explored:

```bash
cargo +nightly fuzz coverage fuzz_parse
```

## What to Look For

The fuzzer helps find:

- **Panics**: Any `unwrap()`, `expect()`, or assertion failure
- **Stack overflows**: From deeply nested constructs
- **Memory exhaustion**: From pathological inputs
- **Infinite loops**: Code that never terminates

All of these are bugs that should be fixed with proper error handling.

## Security Considerations

The compiler processes untrusted input (user ST code). Bugs found here could potentially be exploited by:

1. Malicious logic authors trying to crash the compiler
2. Denial-of-service attacks against compilation endpoints
3. Logic injection through parser confusion

Please report any security-relevant bugs according to the project's security policy.
