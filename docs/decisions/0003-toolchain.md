# 0003. Toolchain

- **Status:** Proposed
- **Date:** 2026-10-15

## Context

NeOS pins stable Rust 1.97.0 and edition 2024
([NeOS ADR 0004](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0004-toolchain-and-build.md)).
Nebu is built for the host, not for `aarch64-unknown-none`.

## Options

### Option A: Track the latest stable independently

There is less to coordinate, but the two projects would drift onto different compilers.

### Option B: Pin the same version as NeOS

Both projects use one compiler, and upgrades happen together.

## Decision

Adopt **Option B**. `rust-toolchain.toml` pins the same version as NeOS (currently
1.97.0), with `rustfmt` and `clippy`. Nebu uses edition 2024. Upgrading the pin is
done in both repositories together.

## Consequences

- One compiler to install for both projects.
- A toolchain upgrade needs two small pull requests, one per repository.
