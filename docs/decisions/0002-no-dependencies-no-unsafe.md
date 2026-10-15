# 0002. No dependencies, no unsafe

- **Status:** Proposed
- **Date:** 2026-10-15

## Context

NeOS writes every line itself
([NeOS ADR 0002](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0002-no-external-dependencies.md))
and confines `unsafe` to a tiny audited core
([NeOS ADR 0003](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0003-unsafe-code-policy.md)).

Nebu differs in one important way: it is an ordinary program on the host, not
bare-metal code. Rust's standard library (`std`) is therefore available. And
unlike the kernel, an emulator never touches real hardware. Guest memory and
registers are just Rust data structures.

## Options

### Option A: Allow crates

Emulators commonly use crates for argument parsing, ELF loading, terminals and
GDB protocol handling. This is quicker, but it goes against the project's ethos.

### Option B: No crates; `std` allowed; no `unsafe` at all

Everything is written in this repository, on top of the Rust standard library.

## Decision

Adopt **Option B**:

- **No external crates.** `Cargo.lock` lists only `nebu` itself.
- **`std` is allowed**, as it ships with the Rust toolchain.
- **No `unsafe`, anywhere.** `unsafe_code = "forbid"` is set in `Cargo.toml` and
  `#![forbid(unsafe_code)]` at the crate root. An emulator has no legitimate need
  for it, so unlike NeOS there is no exception at all.
- Clippy's default lints are set to deny.

## Consequences

- Nebu is an entirely safe Rust program, so any memory-safety bug would be a
  compiler or standard library bug.
- We write our own argument parsing, image loading, terminal handling and GDB
  remote protocol support.
- Performance tricks that need `unsafe`, such as JIT compilation into executable
  memory, are ruled out (see ADR 0004).
