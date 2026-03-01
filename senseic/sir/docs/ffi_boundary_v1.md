# Senseic FFI Boundary v1

This document defines the v1 C ABI surface for the SIR text to bytecode integration.

## Scope

Input:
- UTF-8 SIR source text
- Compile options (`init_only`, entrypoint names, optimization passes)

Output:
- EVM bytecode bytes
- Structured diagnostics (kind + span + message)
- Top-level status + message

## Rust crate

- Crate: `sir/crates/ffi`
- Package name: `senseic-ffi`
- Library name: `senseic_ffi`
- Artifact: `staticlib` (`libsenseic_ffi.a`)

## ABI functions

Version:
- `senseic_abi_version() -> u32`
- `senseic_bytecode_version() -> u32`
- `senseic_compiler_version() -> SenseicByteSlice`

Compile:
- `senseic_compile_options_default() -> SenseicCompileOptions`
- `senseic_compile_sir_to_bytecode(source, options) -> *mut SenseicCompileResult`

Result access:
- `senseic_compile_result_status(result) -> SenseicStatus`
- `senseic_compile_result_message(result) -> SenseicByteSlice`
- `senseic_compile_result_bytecode(result) -> SenseicByteSlice`
- `senseic_compile_result_diagnostic_count(result) -> usize`
- `senseic_compile_result_diagnostic_get(result, index, out) -> u8`
- `senseic_compile_result_free(result)`

## Ownership

- `SenseicCompileResult*` is owned by the caller and must be released with `senseic_compile_result_free`.
- `SenseicByteSlice` values returned by getters borrow memory owned by the result handle.
- Borrowed slices become invalid immediately after `senseic_compile_result_free`.
- The caller must copy any bytes it needs to keep after free.
- `senseic_compiler_version` returns a static slice to process-lifetime memory and does not depend on a result handle.

## Panic and error policy

- Every `extern "C"` entrypoint is wrapped with `catch_unwind`.
- No Rust panic is allowed to unwind across the C ABI.
- Panics are converted to `SenseicStatus::Panic` where a status is available.
- Argument decoding failures (invalid UTF-8, null pointer with non-zero len) return `SenseicStatus::InvalidArgument`.

## Diagnostic model

- `SenseicStatus` is the top-level control-flow result.
- `SenseicDiagnostic` carries structured error payload:
  - `kind`
  - `span_start`, `span_end` (byte offsets in source)
  - `message` (UTF-8 bytes, same as the top-level result message for all diagnostics in v1)

## Thread safety

- Compile calls are re-entrant and have no shared mutable global state.
- A `SenseicCompileResult*` can be read from multiple threads only if the caller synchronizes access and guarantees no concurrent `free`.

## Header generation

The ABI header is generated with cbindgen:

```bash
cbindgen --config sir/crates/ffi/cbindgen.toml --crate senseic-ffi --output sir/crates/ffi/include/senseic_ffi.h
```
