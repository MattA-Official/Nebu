# 0012. Instructions for capabilities

- **Status:** Accepted
- **Date:** 2026-10-19

## Context

NeOS stage 4 gives the root task all free memory as untyped capabilities, and lets
it retype, map and revoke them
([NeOS ADR 0016](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0016-untyped-memory-and-capabilities.md)).
The new code needs no new devices, system registers or exceptions:

- the kernel reaches object memory through the kernel window, which ADR 0009
  already translates;
- it changes user page tables, which ADR 0009 already walks on every access, since
  Nebu has no TLB;
- it flushes the TLB with `tlbi vmalle1` and orders the table writes with `dsb ish`,
  which Nebu already runs.

It does use eleven instructions Nebu did not have. The compiler chose them for the
object code, the splitting of memory into aligned blocks (trailing zeros, through
RBIT and CLZ), and the root task's test:

| Instruction              | Used for                                         |
|--------------------------|--------------------------------------------------|
| BIC, BICS (shifted register) | clearing bits, and testing for bits outside a mask |
| CCMN (immediate)         | conditional comparisons with negative constants  |
| CLZ, RBIT                | counting trailing zeros                          |
| CSINV                    | `csetm`, setting all bits on a condition         |
| EXTR                     | `ror`, rotating by a constant                    |
| LDUR                     | loads with a negative offset                     |
| SBCS                     | `ngcs`, in overflow checks                       |
| SUBS (extended register) | comparing a register with a byte                 |
| UMULH                    | division by a constant, as a multiply            |

## Options

- **Implement every instruction on the same pages**, such as all of ADC, ADCS, SBC
  and SBCS together. Most of them nothing runs, so nothing tests them.
- **Implement exactly what runs**, as ADR 0001 asks, halting on the rest of each
  page. Every instruction Nebu runs is then checked by NeOS's tests against QEMU.

LDUR, like LDURB in ADR 0010, behaves exactly like LDR with an offset, so it can
share LDR's variant.

## Decision

1. The eleven instructions above are implemented, with every form of each: both
   register widths, and for SUBS (extended register) all eight extensions and
   shifts up to 4.
2. **LDUR** decodes as LDR (immediate) with an unscaled offset, as LDURB does with
   LDRB.
3. The rest of each page still halts Nebu: ADC, ADCS and SBC; CCMN (register);
   CSNEG; EON; ADD, ADDS and SUB (extended register); and the other
   data-processing (1 source) instructions.

## Consequences

- Nebu runs all eight of NeOS's test scenarios, including the new `capabilities`
  scenario, with the same output as QEMU.
- Compiler-chosen instructions keep arriving as NeOS grows. Each needs only a
  decoder entry, its execution and a test, and the scenarios catch any that are
  missing.
