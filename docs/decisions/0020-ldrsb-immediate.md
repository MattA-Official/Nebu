# 0020. LDRSB (immediate)

- **Status:** Accepted
- **Date:** 2026-10-21

## Context

NeOS's next stage adds a shell, a program that reads commands from the console
([NeOS ADR 0027](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0027-the-shell.md)).
The kernel needs nothing new from Nebu, but the shell's code compares strings, and
the compiler reads them a byte at a time with `ldrsb w15, [x14], #1`: LDRSB with
an immediate offset, post-indexed. Nebu halts on it, since ADR 0018 implemented
LDRSB only with a register offset.

## Options

- **Change the shell so the compiler picks another instruction.** Nebu's job is to
  run whatever NeOS compiles to (ADR 0001), and any later comparison of strings
  could bring it back.
- **Implement LDRSB (immediate) in its post-indexed form only**, which is all that
  runs. But the four forms of the immediate offset (unsigned, unscaled, pre- and
  post-indexed) share their decoding with LDRB's, which Nebu already runs in every
  form, so leaving three out would add code to refuse them.
- **Implement LDRSB (immediate) in every form** _(chosen)_, sharing LDRB
  (immediate)'s decoding and addressing, as ADR 0018 does for LDRSB (register). It
  is LDRB with the byte sign-extended to 32 or 64 bits.

## Decision

1. LDRSB (immediate) is implemented with an unsigned offset, an unscaled offset
   (LDURSB), and pre- and post-indexing, sign-extending to 32 or 64 bits.
2. LDRSH, and LDRSW with an immediate offset, still halt Nebu.

## Consequences

- Nebu runs all fifteen of NeOS's test scenarios, including the new `shell`
  scenario, with the same output as QEMU.
- Nebu implements 94 instructions, with the SD card stage's.
