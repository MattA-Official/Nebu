# 0050. The mailboxes between cores

- **Status:** Proposed
- **Date:** 2026-11-05
- **Amends:** [0031](0031-exclusives-and-every-cores-timer.md) (decision 3, mailbox
  interrupts stay off)

## Context

[NeOS ADR 0054](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0054-interrupts-between-cores.md)
lets one core interrupt another, through the BCM2836 local block's mailboxes. The
Pi 3 has no other way. The local block gives each core four 32-bit mailboxes, and for
each one:

- a write-set register, at 0x80 plus 16 times the core's number plus 4 times the
  mailbox's, which any core can write to set bits;
- a read and write-clear register, at 0xC0 plus the same, which reads the mailbox and
  clears the bits written to it;
- an enable bit in the core's mailbox interrupt control, at 0x50 plus 4n: bits 0 to 3
  raise an IRQ while that mailbox is not 0, and bits 4 to 7 an FIQ;
- a bit in the core's IRQ source, at 0x60 plus 4n: bits 4 to 7 for mailboxes 0 to 3,
  set while the mailbox is not 0 and its IRQ is enabled.

NeOS uses only mailbox 0 of each core, as an IRQ. It sets bits in another core's
mailbox while it holds the kernel lock, and each core clears its own. Nebu today
accepts only 0 in the mailbox interrupt control (ADR 0031) and has no mailbox
registers.

## Options

### Which mailboxes

- **Mailbox 0 of each core, as an IRQ** _(chosen)_, all that NeOS uses. The write-set
  and write-clear registers of mailboxes 1 to 3, and enabling them or any FIQ, halt
  Nebu, as anything else it does not model does (ADR 0001).
- **All four mailboxes of each core.** Little more code, but code that no test runs,
  and a NeOS change that used another mailbox by mistake would work in Nebu.

### When the IRQ is seen

- **At once** _(chosen)_. Nebu runs the cores one instruction at a time in turn (ADR
  0030), so a mailbox set by one core is seen by the other before that core's next
  instruction, and an IRQ is taken there if it is enabled and unmasked, or ends a
  `wfi`. That is as fast as hardware can be; NeOS must not depend on it being slower.
- **After a delay**, to model the time a write takes to reach the local block. The Pi
  gives no figure, and NeOS's correctness must not depend on one.

QEMU's `bcm2836_control` behaves the same way: the source bit, and the IRQ, follow the
mailbox and its enable bit at once.

## Decision

1. The local block holds mailbox 0 of each core. A write to its write-set register,
   at 0x80 + 16n, sets the bits written; a read of its read and write-clear register,
   at 0xC0 + 16n, returns it, and a write there clears the bits written.
2. A core's mailbox interrupt control, at 0x50 + 4n, may be 0 or 1. With 1, the
   core's IRQ source has bit 4 set while its mailbox 0 is not 0, and the core takes an
   IRQ, or wakes from `wfi`, as it does for its timer.
3. Every other mailbox register, any read of a write-set register, and any other
   value in a mailbox interrupt control halt Nebu.
4. A core waiting in `wfi` whose mailbox IRQ is raised runs again at the next count,
   as for its timer. Only a running core can set a mailbox, so skipping ahead while
   every core waits (ADR 0031) needs no change.

## Consequences

- NeOS's interrupts between cores work in Nebu as on the Pi, and a lost or missing
  signal shows up in Nebu as a thread that wakes late, every run the same way.
- Since the IRQ is seen at once, a race that needs the write to be slow on the Pi does
  not show in Nebu; only the Pi tests that.
