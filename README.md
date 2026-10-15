# Nebu

**Nebuchadnezzar** is a minimal Raspberry Pi 3B emulator, written in pure Rust and
developed in lockstep with [NeOS](https://github.com/MattA-Official/NeOS).

Nebu emulates only the hardware NeOS actually uses. Anything else halts it with a
precise error, so each gap is visible and gets filled exactly when the kernel needs
it. QEMU's `raspi3b` machine is the reference that both projects are checked against.

## Principles

- **Only what is used.** Nebu runs AArch64 code only, implements instructions and
  devices on demand, and fails loudly on anything it does not support
  ([ADR 0001](docs/decisions/0001-scope.md)).
- **Every line is ours, and all of it is safe.** No external crates, and no `unsafe`
  code at all ([ADR 0002](docs/decisions/0002-no-dependencies-no-unsafe.md)).
- **Simple and inspectable.** A fetch, decode and execute interpreter that can
  single-step and trace every instruction ([ADR 0004](docs/decisions/0004-execution-model.md)).
- **Decisions are written down** in [`docs/decisions/`](docs/decisions/README.md).

## Status

A skeleton for now. The CPU core arrives alongside NeOS step 2, which boots a kernel
to a "hello" message over the serial port. What Nebu supports is tracked in
[`docs/implemented.md`](docs/implemented.md).

## Building

Requires [`rustup`](https://rustup.rs). The pinned compiler installs automatically
from `rust-toolchain.toml`.

```sh
cargo build
cargo run -- path/to/kernel8.img
```
