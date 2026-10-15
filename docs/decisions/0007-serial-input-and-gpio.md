# 0007. Serial input and GPIO

- **Status:** Proposed
- **Date:** 2026-10-15

## Context

NeOS step 3 adds a chainloader that receives the kernel over the serial port
([NeOS ADR 0010](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0010-serial-chainloader.md)).
To run it, Nebu's UART must be able to receive bytes, which ADR 0006 ruled out
("there is no input yet"). The chainloader also sets GPIO pins 14 and 15 to the
PL011's function, so Nebu needs the GPIO registers it writes.

`cargo xtask test` drives the chainloader in QEMU through QEMU's standard input
(`-serial stdio`), so Nebu should take input the same way.

## Options

### Where received bytes come from

- **Standard input:** matches QEMU's `-serial stdio`, so `xtask` can drive both
  emulators the same way.
- **A TCP socket or a pseudo-terminal:** closer to a real cable, but more code, and
  the standard library cannot create a pseudo-terminal.

### How input reaches the CPU

- **Read standard input when the CPU polls the UART:** simple, but a read with no
  input waiting would stop the emulated CPU, which real hardware never does.
- **A reader thread:** a separate thread reads standard input and queues the bytes;
  the UART takes them from the queue without waiting.

### The receive FIFO

The real PL011 holds 16 received bytes, and drops any more that arrive before the
software reads them. QEMU instead stops reading its input while the FIFO is full, so
nothing is ever lost.

### GPIO

Nebu has no pins, so routing them changes nothing it can observe. The registers can
either be ignored or store what is written.

## Decision

1. **Input.** A thread reads Nebu's standard input and queues the bytes for the
   PL011. Reading the UART never waits.
2. **Receiving.** FR.RXFE is clear while a byte is queued. Reading DR takes the next
   byte, or gives 0 if there is none, as in QEMU. The queue has no limit, so, as in
   QEMU, no byte is ever lost; this is more forgiving than the hardware. Line errors
   are never reported. FR.BUSY always reads as clear, because bytes are sent
   instantly.
3. **GPIO.** The GPIO controller at 0x3F20_0000 implements GPFSEL1, GPPUD and
   GPPUDCLK0, which store what is written (masked to their defined bits) and read
   it back. Any other GPIO register halts Nebu, like any other unimplemented
   device register.

This amends points 3 (memory map) and 5 (the UART) of ADR 0006.

## Consequences

- `cargo xtask test` sends the kernel to the chainloader in Nebu exactly as it does
  in QEMU, and the two transcripts can be compared byte for byte.
- `cargo xtask run --nebu` now passes what you type to the kernel, which ignores it
  for now.
- Bugs that only appear when the hardware FIFO overflows will not show up in Nebu or
  QEMU. The chainloader's checksum catches lost bytes on real hardware.
