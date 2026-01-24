# Process Image ABI

This document defines the memory layout contract between WebAssembly PLC logic modules and the vPLC runtime host. Wasm modules must adhere to this ABI to correctly exchange I/O data with the fieldbus layer.

## Overview

The process image is a region of Wasm linear memory that the host reads/writes before and after each `step()` call. This provides a memory-mapped interface for I/O access.

```
┌─────────────────────────────────────────────────────────────┐
│                     Wasm Linear Memory                       │
├─────────────────────────────────────────────────────────────┤
│ Offset 0x00 │ Digital Inputs      │  4 bytes (32 bits)      │
│ Offset 0x04 │ Digital Outputs     │  4 bytes (32 bits)      │
│ Offset 0x08 │ Analog Inputs       │ 32 bytes (16 × i16)     │
│ Offset 0x28 │ Analog Outputs      │ 32 bytes (16 × i16)     │
│ Offset 0x48 │ System Info         │  8 bytes                │
│ Offset 0x50 │ Application Memory  │ User-defined            │
└─────────────────────────────────────────────────────────────┘
```

## Memory Layout

### Digital Inputs (Offset 0x00, 4 bytes)

32 digital input bits packed as a little-endian `u32`.

| Bit | Byte 0 (0x00) | Byte 1 (0x01) | Byte 2 (0x02) | Byte 3 (0x03) |
|-----|---------------|---------------|---------------|---------------|
| 0-7 | DI[0..7]      | DI[8..15]     | DI[16..23]    | DI[24..31]    |

**Access:** Read-only during `step()`. Written by host before each cycle.

### Digital Outputs (Offset 0x04, 4 bytes)

32 digital output bits packed as a little-endian `u32`.

| Bit | Byte 0 (0x04) | Byte 1 (0x05) | Byte 2 (0x06) | Byte 3 (0x07) |
|-----|---------------|---------------|---------------|---------------|
| 0-7 | DO[0..7]      | DO[8..15]     | DO[16..23]    | DO[24..31]    |

**Access:** Write during `step()`. Read by host after each cycle.

### Analog Inputs (Offset 0x08, 32 bytes)

16 analog input channels as little-endian signed 16-bit integers (`i16`).

| Offset | Content |
|--------|---------|
| 0x08   | AI[0]   |
| 0x0A   | AI[1]   |
| ...    | ...     |
| 0x26   | AI[15]  |

**Range:** -32768 to +32767 (raw ADC values, scaling is application-defined)

**Access:** Read-only during `step()`. Written by host before each cycle.

### Analog Outputs (Offset 0x28, 32 bytes)

16 analog output channels as little-endian signed 16-bit integers (`i16`).

| Offset | Content |
|--------|---------|
| 0x28   | AO[0]   |
| 0x2A   | AO[1]   |
| ...    | ...     |
| 0x46   | AO[15]  |

**Range:** -32768 to +32767 (raw DAC values, scaling is application-defined)

**Access:** Write during `step()`. Read by host after each cycle.

### System Info (Offset 0x48, 8 bytes)

System information provided by the host.

| Offset | Size | Content              | Type |
|--------|------|----------------------|------|
| 0x48   | 4    | Cycle time (ns)      | u32  |
| 0x4C   | 4    | Flags                | u32  |

**Flags:**
- Bit 0: `FIRST_CYCLE` - Set on the first cycle after initialization

**Access:** Read-only. Written by host before each cycle.

## Byte Order

All multi-byte values use **little-endian** byte order, matching WebAssembly's native memory model.

## Host Functions

In addition to the memory-mapped process image, Wasm modules can import host functions:

```wat
(import "plc" "read_di" (func $read_di (param i32) (result i32)))
(import "plc" "write_do" (func $write_do (param i32 i32)))
(import "plc" "read_ai" (func $read_ai (param i32) (result i32)))
(import "plc" "write_ao" (func $write_ao (param i32 i32)))
(import "plc" "get_cycle_time" (func $get_cycle_time (result i32)))
(import "plc" "get_cycle_count" (func $get_cycle_count (result i64)))
(import "plc" "is_first_cycle" (func $is_first_cycle (result i32)))
(import "plc" "log_message" (func $log_message (param i32 i32)))
```

These functions read from / write to the same process image region. They are provided for convenience and bounds checking.

## Required Exports

Wasm modules must export:

| Export   | Signature  | Description                    |
|----------|------------|--------------------------------|
| `memory` | Memory     | Linear memory (min 1 page)     |
| `step`   | `() -> ()` | Called every scan cycle        |

Optional exports:

| Export   | Signature  | Description                    |
|----------|------------|--------------------------------|
| `init`   | `() -> ()` | Called once before first cycle |
| `fault`  | `() -> ()` | Called when entering fault     |

## Example (WAT)

```wat
(module
  (import "plc" "read_di" (func $read_di (param i32) (result i32)))
  (import "plc" "write_do" (func $write_do (param i32 i32)))

  (memory (export "memory") 1)  ;; 64KB minimum

  (func (export "step")
    ;; Read DI bit 0, write to DO bit 0 (simple pass-through)
    (call $write_do
      (i32.const 0)
      (call $read_di (i32.const 0))
    )
  )
)
```

## Versioning

This document describes **ABI version 1.0**.

Future versions may extend the layout beyond offset 0x50. Modules should not assume any specific values beyond documented offsets.

| Version | Changes                          |
|---------|----------------------------------|
| 1.0     | Initial specification            |

## Compatibility Notes

1. **Minimum memory:** 1 Wasm page (64KB). The process image uses only the first 80 bytes.

2. **Alignment:** All values are naturally aligned. No padding is required.

3. **Atomicity:** The host guarantees that the entire process image is consistent within a single cycle (no partial updates).

4. **Endianness:** Little-endian throughout. This matches native Wasm memory access.
