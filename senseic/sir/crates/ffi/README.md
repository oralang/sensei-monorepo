# senseic-ffi

Thin C ABI for the `SIR text -> bytecode` pipeline.

## Build

```bash
cargo build -p senseic-ffi
```

This crate exports a `staticlib` named `libsenseic_ffi.a`.

## Header generation

```bash
cbindgen --config sir/crates/ffi/cbindgen.toml --crate senseic-ffi --output sir/crates/ffi/include/senseic_ffi.h
```

The generated header is intended to be consumed by Zig via `@cImport`.
