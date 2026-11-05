# 0042. DMB, ADC and SMULH

- **Status:** Proposed
- **Date:** 2026-11-05

## Context

NeOS's clock service keeps the date in a time frame that every program can read
([NeOS ADR 0081](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0081-the-wall-clock.md)).
The frame holds two copies and an index, read and written with acquire loads and
release stores so that a reader never sees half a write, and each copy holds a line
from the counter to the date, worked out in 128 bits. The compiler turns these into
three instructions Nebu halts on:

- `dmb ishld` and `dmb ish`: DMB, the data memory barrier. Nebu has DSB and ISB.
- `smulh`: SMULH, the high half of a signed 64-bit product. Nebu has UMULH.
- `adc`: ADC, add with carry. Nebu has SBC and SBCS.

## Options

- **Change NeOS so the compiler picks other instructions,** such as load-acquire
  (LDAR) for each word, or unsigned arithmetic. The frame's reader is right as
  written, and any atomic with an ordering or any `i128` sum could bring these
  back. Nebu's job is to run what NeOS compiles to (ADR 0001).
- **Implement the three** _(chosen)_, each decoded with its neighbours.

## Decision

1. DMB is implemented with all sixteen values of its option field, and does nothing.
   Nebu runs one instruction at a time on one core after another, and models no
   caches or write buffers, so every core already sees every access in program
   order, as with DSB.
2. SMULH writes the high 64 bits of the signed 128-bit product.
3. ADC is implemented in both widths: `rd = rn + rm + C`, leaving the flags alone.
   ADCS, which sets them, is not, since nothing uses it yet.

## Consequences

- Nebu runs every NeOS scenario now that every program has the time frame mapped,
  including the new `clock` scenario.
- Nebu cannot find a missing barrier in NeOS: code that leaves one out runs as if it
  were there. Only QEMU with several threads, or a Pi, can show that. This is no
  change from DSB.
