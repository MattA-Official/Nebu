# 0044. LDRSH (immediate)

- **Status:** Accepted
- **Date:** 2026-11-06

## Context

NeOS's accounts service keeps the people who can log in, and checks their
passwords
([NeOS ADR 0080](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0080-users-and-logging-in.md)).
Its code reads a signed 16-bit value from the stack with `ldrsh w8, [sp, #8]`:
LDRSH with an unsigned immediate offset, which ADR 0020 left out. It also swaps the
bytes of words with `rev`, which ADR 0043 brought in for time zones.

## Options

- **Change the service so the compiler picks another instruction.** Nebu's job is
  to run whatever NeOS compiles to (ADR 0001), and any `i16` could bring it back.
- **Implement the unsigned offset only**, which is all that runs. As with LDRSB,
  the four forms share their decoding with LDRH's, which Nebu already runs in every
  form, so leaving three out would add code to refuse them.
- **Implement LDRSH (immediate) in every form** _(chosen)_, as ADR 0020 did for
  LDRSB: LDRH with the halfword sign-extended to 32 or 64 bits.

## Decision

1. LDRSH (immediate) is implemented with an unsigned offset, an unscaled offset
   (LDURSH), and pre- and post-indexing, sign-extending to 32 or 64 bits.
2. LDRSH (register) still halts Nebu.

## Consequences

- Nebu runs NeOS's new `accounts` scenario with the same output as QEMU.
