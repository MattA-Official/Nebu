# 0044. LDRSH (immediate) and REV

- **Status:** Proposed
- **Date:** 2026-11-06

## Context

NeOS's accounts service keeps the people who can log in, and checks their
passwords
([NeOS ADR 0080](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0080-users-and-logging-in.md)).
Its code brings two instructions Nebu halts on:

- `ldrsh w8, [sp, #8]`: LDRSH with an unsigned immediate offset, which ADR 0020
  left out.
- `rev x8, x8`: REV, which reverses the order of the bytes in a register, from
  Argon2's and SHA-3's reading of big- and little-endian words.

## Options

- **Change the service so the compiler picks other instructions.** Nebu's job is
  to run whatever NeOS compiles to (ADR 0001), and any `i16` or byte swap could
  bring them back.
- **Implement only the forms that run**: LDRSH's unsigned offset, and REV in 64
  bits. As with LDRSB,
  the four forms share their decoding with LDRH's, which Nebu already runs in every
  form, so leaving three out would add code to refuse them. REV16, REV32 and REV
  differ only in the size of the piece whose bytes are reversed.
- **Implement LDRSH (immediate) in every form, and REV16, REV32 and REV in both
  widths** _(chosen)_. LDRSH is LDRH with the halfword sign-extended, as ADR 0020
  did for LDRSB.

## Decision

1. LDRSH (immediate) is implemented with an unsigned offset, an unscaled offset
   (LDURSH), and pre- and post-indexing, sign-extending to 32 or 64 bits.
2. REV16, REV32 (64 bits only) and REV are implemented in both widths, decoded
   with RBIT and CLZ, as one instruction that reverses the bytes in each piece of
   2, 4 or 8 bytes.
3. LDRSH (register), and LDRSW with an immediate offset, still halt Nebu.

## Consequences

- Nebu runs NeOS's new `accounts` scenario with the same output as QEMU.
