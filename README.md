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

Nebu runs NeOS up to its user-space SD card driver. It boots the kernel directly or
through the NeOS chainloader, drops to EL1 with it, translates every access through
the kernel's page tables, answers the kernel's questions about memory and the screen
through the firmware mailbox, keeps a framebuffer it can save as an image, runs
the kernel's threads at EL0 with their system calls,
preempts them with the virtual timer's interrupt, wakes its drivers with the
system timer's, the UART's, the SD card controller's and the USB controller's,
reads blocks from an SD card image, runs USB transfers to a hub and keyboard, and takes the exceptions and aborts the kernel's tests raise. Its output matches QEMU's byte for byte, including
the panic path and the kernel's fault reports. It currently implements 95 A64
instructions, 27 system registers, six kinds of exception and seven devices; see
[`docs/implemented.md`](docs/implemented.md).

## Building and running

Requires [`rustup`](https://rustup.rs). The pinned compiler installs automatically
from `rust-toolchain.toml`.

```sh
cargo build --release
./target/release/nebu path/to/kernel8.img
```

| Option           | Effect |
|------------------|--------|
| `--semihosting`  | Lets `hlt #0xf000` make semihosting calls, as QEMU's `-semihosting` does. NeOS test builds use this to exit. |
| `--trace`        | Prints every instruction to standard error before running it. |
| `--sd <file>`    | Puts an SD card holding `<file>`, which must be a whole number of 512 KiB, larger than 2 GiB and at most 32 GiB, in the slot. It is only ever read ([ADR 0019](docs/decisions/0019-the-sd-card.md)). |
| `--load <file>@<address>` | Copies `<file>` into memory at `<address>` (hexadecimal, with `0x`) before starting, as the firmware does with `config.txt`'s `initramfs` and QEMU with `-device loader`. It may be repeated ([ADR 0022](docs/decisions/0022-loading-a-second-file.md)). |
| `--screen <file.ppm>` | When the machine stops, saves what the screen shows to `<file.ppm>`, as QEMU's `screendump` does ([ADR 0023](docs/decisions/0023-the-framebuffer.md)). |
| `--keyboard` | Plugs a USB keyboard into the USB port, behind a hub, as QEMU's `-device usb-kbd` does ([ADR 0024](docs/decisions/0024-the-usb-controller-and-keyboard.md)). |
| `--keys <list>` | Plugs in the keyboard and types on it, a press (`+name`) or release (`-name`) each time NeOS polls it, with QEMU's names for keys ([ADR 0025](docs/decisions/0025-typing-on-the-keyboard.md)). |

The emulated serial port is Nebu's standard output and standard input, like QEMU's
`-serial stdio`.

Usually you won't run Nebu directly: NeOS's `cargo xtask run --nebu` and
`cargo xtask test` build and run it for you from a checkout next to NeOS.

When Nebu meets something it does not implement, it stops and says exactly what:

```text
nebu: stopped at pc 0x000801bc: cannot run instruction 0xd65f03c0: instruction not implemented
  x0  0x00000000000902b8   x1  0x00000000000802d0
  ...
```

## Layout

| Path             | What it is                                              |
|------------------|---------------------------------------------------------|
| `src/decode.rs`  | Turns instruction words into `Instruction` values, and prints them. |
| `src/cpu.rs`     | Registers, flags and the execution of each instruction. |
| `src/mmu.rs`     | Address translation: the page table walk.               |
| `src/bus.rs`     | The physical address space: sparse RAM and the device map. |
| `src/pl011.rs`   | The PL011 UART.                                         |
| `src/gpio.rs`    | The GPIO controller's pin function registers.           |
| `src/mailbox.rs` | The VideoCore mailbox and the firmware's answers.       |
| `src/local.rs`   | The local block and the BCM2835 interrupt controller.   |
| `src/system_timer.rs` | The BCM2835 system timer.                          |
| `src/main.rs`    | Command-line handling and the run loop.                 |

## Licence

[MIT](LICENSE).
