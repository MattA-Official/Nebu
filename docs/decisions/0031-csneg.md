# 0031. CSNEG

- **Status:** Proposed
- **Date:** 2026-11-03

## Context

NeOS's random service test counts how often each byte value comes up over a
thousand reads, and checks that none is far from the average, with Rust's
`u32::abs_diff` (NeOS ADR 0047). The compiler turns that into a subtraction
followed by `csneg w9, w9, w9, hi`: negate the difference unless the first
operand was the larger. Nebu stops on it, since it decodes the other three
conditional selects (CSEL, CSINC and CSINV) but not this one.

Nebu implements only what NeOS uses (ADR 0002), so the question is not whether
CSNEG is worth having in general, but how to get past this one instruction.

## Options

- **Write the test without `abs_diff`**, comparing the count against the average's
  two bounds instead. It would need no change to Nebu. But the compiler is free to
  choose this instruction for any such comparison, so the next piece of arithmetic
  could bring it back, and NeOS would be written around its emulator rather than the
  other way round.
- **Implement CSNEG** _(chosen)_. It is the fourth of a family Nebu already decodes
  in one place: the same fields, one more pair of bits, and the value negated instead
  of inverted. It is a handful of lines, and it is a real instruction on the
  hardware, so NeOS is free to use whatever the compiler picks.

## Decision

1. **CSNEG** is decoded in the conditional select group, as `(op, o2) = (1, 1)`,
   and executed as `rd = if condition holds { rn } else { -rm }`, at both widths.
   The pair `(0, 1)` with `op` 0 stays unallocated, as the architecture says.
2. **Tests** check that it decodes and prints as `csneg`, including the exact word
   the random service's test compiles to.

## Consequences

- Nebu runs the random service's test, and anything else the compiler writes
  `abs_diff` or a conditional negation into.
- The conditional select group is now complete, so no instruction in it can stop
  Nebu again.
