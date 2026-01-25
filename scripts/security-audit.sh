#!/usr/bin/env bash
set -euo pipefail

TOOLCHAIN="${TOOLCHAIN:-1.88.0}"

if ! command -v rustup >/dev/null 2>&1; then
  echo "rustup is required to run security audits." >&2
  exit 1
fi

rustup toolchain install "${TOOLCHAIN}"

cargo +"${TOOLCHAIN}" install cargo-deny --version 0.19.0 --locked
cargo +"${TOOLCHAIN}" install cargo-audit --version 0.22.0 --locked

cargo +"${TOOLCHAIN}" deny check
# Clear any stale/non-git advisory db and re-fetch.
rm -rf "${CARGO_HOME:-$HOME/.cargo}/advisory-db"
cargo +"${TOOLCHAIN}" audit --file .cargo/audit.toml
