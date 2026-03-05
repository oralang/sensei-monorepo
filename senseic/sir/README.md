# Sensei Intermediate Representation (SIR)

SIR is an IR tailored for targeting the EVM. It's goal is to offer fast compilation via its debug
backend and highly optimized code generation via its "max" backend (name TBD).

## Building SIR Only (No Frontend/Tree-sitter)

Use the SIR sub-workspace manifest:

```sh
cargo build --manifest-path senseic/sir/Cargo.toml -p sir-cli --release
```
