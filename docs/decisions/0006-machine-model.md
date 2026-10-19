# 0006. Machine model

- **Status:** Accepted; points 3 and 5 amended by [0007](0007-serial-input-and-gpio.md); exceptions added by [0008](0008-exception-levels-and-exceptions.md)
- **Date:** 2026-10-15

## Context

To run NeOS step 2 ([NeOS ADR 0008](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0008-early-boot-and-console.md)),
Nebu needs a CPU, memory, one UART and a way for test builds to exit
([NeOS ADR 0009](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0009-emulator-test-harness.md)).
Each of these involves choices about how closely to follow the real Pi 3 and where to
follow QEMU instead, since QEMU is the reference (NeOS ADR 0006).

## Options

### Cores

- **Four cores:** faithful, but QEMU holds cores 1 to 3 in its own spin loop
  before NeOS runs, so they never execute NeOS code.
- **Core 0 only:** matches what NeOS can observe today.

### Memory

- **A 1 GiB array:** simple, but allocates a gigabyte up front.
- **Sparse pages:** RAM is allocated in 64 KiB pages the first time each is written,
  and untouched memory reads as zero, as QEMU's RAM does.

### Alignment

With the MMU off, every data access is to Device memory, which must be aligned. Real
hardware raises an alignment fault on an unaligned access. QEMU is more lenient.

### Waiting for events

NeOS parks cores with `wfe`. Nebu has no devices that can send an event yet.

## Decision

1. **One core.** Nebu emulates core 0 only. Its `MPIDR_EL1` reads `0x8000_0000`, as
   in QEMU.
2. **Starting state.** As in QEMU `raspi3b` with `-kernel`: `kernel8.img` is loaded
   at 0x80000 and the PC starts there. The core is at EL2 using SP_EL2. QEMU's boot
   stub leaves x0 = 0x100 (where a device tree would be), x4 = 0x80000 and the Z flag
   set; every other register, including SP, is zero. These values were read from
   QEMU 8.2's `-d cpu` log at the kernel's first instruction.
3. **Memory map.** RAM covers 0 to 0x3F00_0000, stored as sparse pages. The PL011
   UART's registers are at 0x3F20_1000. Any other address halts Nebu.
4. **Accesses.** Every data access must be aligned to its size, and any other
   access halts Nebu with an alignment error. This is stricter than QEMU and the
   same as the hardware. Instructions can only be fetched from RAM. A load or
   store based on SP while SP is not 16-byte aligned halts Nebu, because whether
   hardware faults depends on SCTLR_EL2.SA, which is not modelled yet. Device
   registers support 32-bit accesses only.
5. **The UART.** Bytes written to the data register go straight to Nebu's standard
   output. The transmit FIFO always reads as empty and so does the receive FIFO
   (there is no input yet). As in QEMU, bytes are sent even if the UART is not
   enabled.
6. **Events.** `wfe` with the event register clear puts the core to sleep. Since
   nothing can wake it, Nebu says so and waits until it is killed, as QEMU does.
7. **Semihosting.** Off by default. With `--semihosting`, `hlt #0xf000` performs a
   semihosting call; only `SYS_EXIT` is implemented. As in QEMU, a reason other
   than `ADP_Stopped_ApplicationExit` exits with code 1. Without `--semihosting`,
   `hlt` halts Nebu.
8. **Exit codes.** The kernel's exit code is passed through from semihosting. When
   Nebu meets something it cannot emulate, it prints the reason and all registers,
   then exits with code 70.
9. **Tracing.** `--trace` prints each instruction's address, encoding and
   disassembly before running it.

## Consequences

- Nebu's output for NeOS step 2 is byte-for-byte identical to QEMU's.
- Exceptions are not modelled at all yet. Anything that would raise one halts Nebu
  with an explanation instead. Exception levels, vectors and `eret` will come with
  the NeOS change that needs them.
- Stricter alignment means Nebu can catch bugs QEMU would let through.
