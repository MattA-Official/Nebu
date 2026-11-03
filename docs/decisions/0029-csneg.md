# 0029. CSNEG

- **Status:** Proposed
- **Date:** 2026-11-03

## Context

NeOS's file system server learns to write
([NeOS ADR 0032](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0032-writing-to-the-file-system.md)).
The compiler turns one of its comparisons into `cneg x8, x8, ne`, an alias of
`csneg x8, x8, x8, eq`: CSNEG, conditional select negation. Nebu halts on it, since
it implements the other three conditional selects, CSEL, CSINC and CSINV, but not
this one.

## Options

- **Change the server so the compiler picks another instruction.** Nebu's job is to
  run whatever NeOS compiles to (ADR 0001), and any `cneg` could bring it back.
- **Implement CSNEG** _(chosen)_, decoded with the other conditional selects, in both
  widths.

## Decision

1. CSNEG is implemented in both widths: `rd` is `rn` if the condition holds, and `rm`
   negated if not.

## Consequences

- Nebu runs NeOS's `writing` scenario.
- Nebu implements 96 instructions, and all four conditional selects.
