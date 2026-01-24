# Virtual PLC (vPLC) — Repo Scaffold

This repository is a **starter scaffold** for building a production-grade Virtual PLC runtime (vPLC). It is intentionally minimal, so you can wire it into your own architecture and add the real-time and fieldbus specifics.

## What this scaffold contains

- A Rust workspace under `crates/` split by concern:
  - `plc-runtime` (cyclic scheduler + process image)
  - `plc-fieldbus` (fieldbus abstraction layer)
  - `plc-compiler` (IEC 61131-3 → Wasm compiler pipeline placeholder)
  - `plc-stdlib` (IEC standard function block placeholders)
  - `plc-daemon` (main binary)
  - `plc-web-ui` (control-plane placeholder)
  - `plc-common` (shared types)

- Documentation placeholders under `docs/`
- Container placeholders (`Dockerfile`, `docker-compose.yml`)
- A basic CI workflow (`.github/workflows/ci.yml`)

## Quick start (dev only)

```bash
cargo test -q
cargo run -q -p plc-daemon -- --help
```

## Next steps

Use the accompanying technical blueprint (provided in the ChatGPT response) to:
- decide on the real-time host model (PREEMPT_RT + CPU/cache isolation),
- implement the fieldbus plane (EtherCAT master + process image),
- implement Wasm sandboxing and deterministic execution,
- add verification and acceptance criteria (cyclictest/HIL/soak).

## License

MIT OR Apache-2.0 (pick for your project).
