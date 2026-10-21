# 0018. The UART's interrupts

- **Status:** Proposed
- **Date:** 2026-10-21
- **Amends:** [0007](0007-serial-input-and-gpio.md) (the UART raises no interrupts)
  and [0017](0017-device-interrupts-and-the-system-timer.md) (decision 2, which
  interrupts the controller enables, and decision 4, how `wfi` waits)

## Context

NeOS stage 7 moves the console into a user-space serial driver
([NeOS ADR 0023](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0023-the-serial-driver-in-user-space.md),
in NeOS #18). The kernel still writes its own diagnostics to the PL011 by polling.
The driver:

- clears every interrupt through ICR (0x44), sets the FIFO levels in IFLS (0x34) to
  half full, and enables the receive, transmit and receive timeout interrupts in
  IMSC (0x38), the transmit one only while it has bytes left over;
- on its interrupt, reads MIS (0x40), reads DR until FR says the receive FIFO is
  empty, and clears what it handled;
- writes DR only when FR says the transmit FIFO has room, and polls FR.BUSY before
  answering a Flush;
- has its interrupt, 57, enabled on the BCM2835 controller through the second
  enable register (0x3F00_B214, bit 25);
- leaves the kernel to idle in `wfi` while it waits for input, with no timer set.

NeOS's new `console` test has `cargo xtask test` type a line on the emulated serial
port once the output shows a prompt, and checks the driver reads it.

Two instructions that Nebu has never run come with it, from the compiler: STRB
(register), as the driver stores into its buffers, and LDRSB (register), as the
root task checks that a string of digits is UTF-8.

**QEMU 8.2's PL011**, the reference, raises the receive interrupt when a byte
arrives with none waiting, whatever the FIFO level says (its source notes that
Linux reads the FIFO only when interrupted), and lowers it when the last byte is
read. It never raises the receive timeout. It sends each byte as it is written, so
its transmit FIFO is never full, and raises the transmit interrupt after every
byte. Its interrupt line is high while RIS and IMSC share a bit. **The Pi's PL011**
raises the receive interrupt at the FIFO level, the timeout after 32 bit periods
of quiet, and the transmit interrupt as the FIFO drains past its level.

## Options

### How the UART raises its interrupts

- **As the Pi does**, with FIFO levels, a timeout and a draining transmit FIFO. It
  would test the driver's handling of levels and timeouts, but Nebu would need to
  model time on the serial line, which neither it nor QEMU does, and the two
  emulators' runs would differ.
- **As QEMU does** _(chosen)_. Receive is raised by the first byte to arrive with
  none waiting and lowered by reading the last, or by ICR; transmit is raised by
  every byte sent; the timeout never. IFLS is stored and read back, but changes
  nothing. A driver that is right on the Pi is right here too, since it must
  already read until the FIFO is empty and never rely on a FIFO level.
- **Only the receive interrupt.** The driver enables transmit only when the FIFO is
  full, which never happens in Nebu, so it would never be used. But a driver that
  enabled it at another time would hang in Nebu and not in QEMU.

Enabling any other interrupt in IMSC, such as the error interrupts, halts Nebu,
since nothing would raise them (ADR 0001).

### When input arrives

Input comes from a thread reading standard input (ADR 0007), so it arrives at a
time Nebu does not choose.

- **Look for it before every instruction.** The interrupt would be raised as soon as
  possible, but the check would slow every instruction down.
- **Only when the UART is read**, as today. The interrupt would never be raised by
  input arriving while the driver waits for it.
- **Every 4096 counts of the core's counter, and whenever the UART is read**
  _(chosen)_: about every 65 µs of the emulated machine's time. The check costs
  nothing that shows, and no one typing notices the delay.

Either way, when input arrives decides only when the driver reads it, not what any
program prints.

### What `wfi` does with nothing due

ADR 0017 has the core sleep for ever when `wfi` finds no timer or comparator due.
With the driver waiting, the kernel now runs `wfi` with nothing due but input.

- **Keep sleeping for ever.** The `console` test would hang.
- **Count on, an instruction's time at a time, until input arrives.** The counter
  would show the wait, but spinning would keep a host core busy for as long as no
  one types.
- **Wait for input, if the UART's receive interrupt and interrupt 57 are both
  enabled** _(chosen)_, without moving the counter. Nebu blocks until standard input
  delivers bytes, which raise the interrupt. If standard input has ended, nothing
  can come, and the core sleeps for ever as before.

### STRB and LDRSB (register)

- **Change NeOS so the compiler picks other instructions.** Nebu's job is to run
  what NeOS compiles to (ADR 0001), as ADR 0015 found for LDRSW.
- **Implement STRB (register) and LDRSB (register)** _(chosen)_, each with every
  extension of the offset register, and LDRSB with both widths, since they are one
  instruction page each (`docs/implemented.md`). The other sign-extending byte
  loads, with immediate offsets, still halt Nebu.

## Decision

1. **The PL011** implements IFLS (read and write, reset 0x12, changing nothing), RIS
   and MIS (read), and ICR (write: clears the raised interrupts named). It raises
   the receive interrupt when a byte arrives with none waiting, lowers it when the
   last waiting byte is read, and raises the transmit interrupt whenever a byte is
   written to DR. IMSC accepts only the receive, transmit and receive timeout
   interrupts; any other bit halts Nebu.
2. **Input** is taken from standard input's queue whenever the UART is read, and
   every 4096 counts of the core's counter.
3. **The BCM2835 controller** also enables interrupt 57, the UART's, through its
   second enable register, and its second pending register shows it while the UART's
   line is high: while RIS and IMSC share a bit. Enabling any other interrupt there still halts Nebu.
4. **`wfi`** with no timer or comparator due waits for standard input if the UART's
   receive interrupt and interrupt 57 are enabled and no received byte is waiting,
   and then finds the interrupt pending. Otherwise, or once standard input has
   ended, the core sleeps for ever.
5. **STRB (register) and LDRSB (register)** are implemented, with every extension of
   the offset register, and LDRSB with 32-bit and 64-bit destinations.

## Consequences

- Nebu runs all thirteen of NeOS's test scenarios, including the new `console`
  scenario, with the same output as QEMU.
- `cargo xtask run --nebu` passes what you type to the serial driver.
- A driver's handling of FIFO levels and the receive timeout is tested only on the
  Pi, as it is in QEMU.
- A run that waits for input waits for as long as it takes: nothing in Nebu times
  out, as before.
- Nebu implements 92 instructions, 27 system registers, six kinds of exception and
  six devices.
