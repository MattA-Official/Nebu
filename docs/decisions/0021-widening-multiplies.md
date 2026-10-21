# 0021. Widening multiplies

- **Status:** Proposed
- **Date:** 2026-10-21

## Context

NeOS's file system stage adds a file system server, a program that reads NeOS's
own file system from the SD card
([NeOS ADR 0029](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0029-the-file-system.md)).
The kernel needs nothing new from Nebu, but the server's code multiplies two 32-bit
numbers into a 64-bit one, and the compiler does that with `umull x11, w24, w19`,
which is UMADDL with the zero register as the addend. Nebu halts on it.

UMADDL has three siblings: UMSUBL subtracts the product instead of adding it, and
SMADDL and SMSUBL sign-extend the 32-bit operands instead of zero-extending them.
All four are one encoding apart in the data-processing (3 source) group, where Nebu
already runs MADD, MSUB and UMULH.

## Options

- **Change the server so the compiler picks another instruction**, such as by
  widening the numbers to 64 bits first. Nebu's job is to run whatever NeOS compiles
  to (ADR 0001), and any later multiplication of two 32-bit numbers could bring it
  back.
- **Implement UMADDL only**, the one that runs. Each is its own instruction page,
  and Nebu implements only what NeOS uses, but its siblings differ from it by one bit
  each, and leaving them out would mean code to refuse them.
- **Implement all four widening multiplies** _(chosen)_, as one instruction with two
  flags, as ADR 0020 did for LDRSB's forms. `smull`, `umull`, `smnegl` and `umnegl`
  are their aliases.

## Decision

1. SMADDL, SMSUBL, UMADDL and UMSUBL are implemented: `rd = ra ± rn × rm`, with `rn`
   and `rm` 32-bit registers, sign- or zero-extended to 64 bits, and `rd` and `ra`
   64-bit registers.
2. SMULH, and the other multiplies of the group, still halt Nebu.

## Consequences

- Nebu runs all seventeen of NeOS's test scenarios, including the new `files`
  scenario, with the same output as QEMU.
- Nebu implements 98 instructions.
