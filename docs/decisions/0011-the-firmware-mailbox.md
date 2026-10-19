# 0011. The firmware mailbox

- **Status:** Proposed
- **Date:** 2026-10-19

## Context

NeOS stage 4 gives user space untyped memory, so the kernel first needs to know how
much RAM there is
([NeOS ADR 0015](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0015-finding-out-how-much-memory-there-is.md)).
It asks the VideoCore firmware through the mailbox's property interface:

1. It builds a message in RAM holding one tag, GET_ARM_MEMORY (0x00010005).
2. It cleans the message out of the data cache with `dc civac`, using the cache line
   size from CTR_EL0, and writes the message's bus address, with channel 8 in the
   low 4 bits, to mailbox 1's WRITE register at 0x3F00B8A0.
3. It waits for mailbox 0's STATUS register (0x3F00B898) to stop saying "empty",
   reads the same word back from its READ register (0x3F00B880), and reads the
   answer out of the message, after invalidating it from the cache again.

On QEMU's `raspi3b`, the firmware answers that the CPU has 960 MiB at address 0:
1 GiB less the 64 MiB QEMU gives the GPU.

So Nebu needs the mailbox, `dc civac`, CTR_EL0, and MSUB, which the kernel's new
decimal printing uses.

## Options

### When the firmware answers

- **After a delay**, as real firmware does. Nothing in NeOS depends on the delay, and
  modelling one means modelling time, which Nebu does not do yet.
- **At once**, during the write to WRITE, as QEMU does. The answer is in RAM and in
  mailbox 0 before the next instruction runs.

### Which tags

- **The property interface in general**, with many tags. Each needs an answer that
  matches QEMU's, and NeOS uses one.
- **Only GET_ARM_MEMORY**, halting on any other tag or channel (ADR 0001).

### The data cache

Nebu has no caches (ADR 0006), so `dc civac` has nothing to do. On hardware, though,
it translates its address and can take a translation fault.

- **A no-op**: simplest, but a kernel bug that cleans an unmapped address would only
  show up in QEMU or on the Pi.
- **Translate the address and do nothing else**: the same faults as hardware.

## Decision

1. **The mailbox registers** sit at 0x3F00B880 to 0x3F00B8BF. Nebu implements three,
   all 32 bits wide:
   - mailbox 0's READ (offset 0x00) returns the oldest answer, and halts Nebu if
     there is none;
   - mailbox 0's STATUS (offset 0x18) has bit 30 (empty) set when there is nothing
     to read, and never bit 31 (full);
   - mailbox 1's WRITE (offset 0x20) sends a message.

   Any other register halts Nebu.
2. **The firmware answers at once.** A write to WRITE must name channel 8, the
   property interface. Nebu drops the top two bits of the bus address, which only
   choose how the GPU caches it, answers the message in RAM, and queues the written
   word for READ.
3. **Only GET_ARM_MEMORY is answered**, as QEMU answers it: base 0, size 0x3C000000,
   with the tag's response code 0x80000008 and the message's 0x80000000. Any other
   tag, or a message without an end tag, halts Nebu.
4. **DC CIVAC** translates its address as a read, so that a bad address faults as it
   would on hardware, and then does nothing. It halts Nebu at EL0, where
   SCTLR_EL1.UCI makes it trap.
5. **CTR_EL0** reads as 0x84448004, QEMU's value for the Cortex-A53: 64-byte cache
   lines. It can be read at EL1 and above only, since SCTLR_EL1.UCT, which would let
   EL0 read it, cannot be set.
6. **MSUB** is implemented, alongside MADD.

## Consequences

- Nebu runs every NeOS test scenario again, and reports the same 960 MiB as QEMU.
- The mailbox is the first device that reads and writes RAM itself. The bus finishes
  the request, since it owns RAM, so the mailbox stays a small device of its own.
- When NeOS asks the firmware for anything else, such as the board's revision or a
  framebuffer, each tag needs adding here, and its answer checking against QEMU's.
