# 0031. Exclusive loads and stores, and every core's timer

- **Status:** Accepted
- **Date:** 2026-11-04
- **Amends:** [0016](0016-the-timer-and-interrupts.md) (only core 0's timer reaches the
  local block) and [0030](0030-four-cores.md) (decision 5, only core 0 takes interrupts)

## Context

[NeOS ADR 0053](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0053-the-kernel-lock-and-a-run-loop-on-every-core.md)
lets every core run threads. For that, the kernel:

- takes a ticket lock, whose next ticket it takes with an atomic add. The Pi 3's
  Cortex-A53 is Armv8.0, with no single-instruction atomics, so Rust compiles the add
  to a loop of `ldxr` and `stxr` (or `ldaxr` and `stlxr`), retried until the store
  succeeds. User programs' atomics compile the same way;
- on every core, sets its virtual timer and enables that timer's interrupt in the
  local block, at 0x40 plus 4 times the core's number, turns its mailbox interrupts
  off at 0x50 plus 4n, and reads its pending sources at 0x60 plus 4n;
- takes IRQs from the virtual timer on every core, and the BCM2835's line on core 0.

Nebu decodes none of the exclusives, models only core 0's registers in the local
block, and halts if any other core enables its timer (ADR 0030).

An exclusive load (`ldxr`) marks the address in the core's exclusive monitor. The
matching exclusive store (`stxr`) writes, and returns 0, only if the monitor still holds
that address; otherwise it writes nothing and returns 1. The monitor is cleared by the
store, by `clrex`, by an exception or an exception return, and by any other core's
store to the same "exclusives reservation granule", which on the A53 is 64 bytes.

## Options

### How the monitor works

- **A monitor for each core, in the bus** _(chosen)_. Each holds the 64-byte granule its
  last exclusive load marked, or nothing. Every store to RAM, from any core, clears
  every monitor that holds the granule it writes to; an exclusive store succeeds only
  if its core's monitor holds its granule, and then clears it in every core. Since the
  cores take turns one instruction at a time (ADR 0030), another core's store between
  a core's `ldxr` and its `stxr` is always seen, and the `stxr` fails as it would on
  hardware.
- **Exclusive stores that always succeed.** Correct with one core, but with four, two
  cores taking a ticket at the same time could take the same one, which is exactly
  what the kernel's lock must prevent. A test would pass in Nebu that fails on the Pi.
- **A monitor that watches only the exact address.** On hardware a store anywhere in
  the granule clears it, so code that works only because of the exact address would
  fail on the Pi.

A core's own ordinary store to its granule also clears its monitor. The architecture
leaves that to the implementation; clearing is the strict choice, and NeOS never
stores between its `ldxr` and `stxr`.

### Each core's timer

- **The local block models all four cores' timer interrupt control and IRQ source**
  _(chosen)_. Each core's virtual timer reaches its own IRQ source when its control
  enables it, and only core 0's source has the BCM2835's line. Each core takes its own
  IRQs and wakes from `wfi` for them. Mailbox interrupts must still stay off, until
  the mailboxes have an ADR of their own.

## Decision

1. Nebu decodes `ldxr`, `ldaxr`, `stxr`, `stlxr` (32 and 64 bits, and the byte and
   halfword forms), and `clrex`.
2. Each core has an exclusive monitor, which holds one 64-byte granule. Any store to
   RAM clears every monitor holding its granule; an exclusive store succeeds only if
   its own core's monitor holds the granule. An exception, `eret` and `clrex` clear the
   core's monitor. An exclusive access must be aligned, as on hardware.
3. The local block has every core's timer interrupt control, mailbox interrupt control
   (which must stay 0) and IRQ source. Each core's virtual timer raises that core's
   IRQ. Only core 0 has the BCM2835's line.
4. Every core takes interrupts. Nebu skips ahead, while every core waits, to the
   first interrupt due on any core.

## Consequences

- The kernel's lock, and programs' atomics, work in Nebu as they do on the Pi, and a
  lock that is wrong for four cores fails in Nebu too, every run the same way.
- Every store to RAM checks four monitors, a small cost on each one.
- The mailboxes, for ADR 0054, take the next free number, since ADR 0051 had kept 0031
  for them.
