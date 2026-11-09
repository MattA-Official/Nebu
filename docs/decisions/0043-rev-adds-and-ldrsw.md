# 0043. REV, ADDS (extended register) and LDRSW (immediate)

- **Status:** Accepted
- **Date:** 2026-11-05

## Context

NeOS's shell now shows dates in local time
([NeOS ADR 0081](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0081-the-wall-clock.md),
decision 8). Each zone's rules are a TZif file (RFC 9636), whose numbers are
big-endian, and the shell reads them with `from_be_bytes`, which the compiler turns
into `rev`. NeOS's TZif reader is the first big-endian format it has read;
everything before was little-endian, as AArch64 is. Two more instructions Nebu halts
on come with it:

- `adds w12, w12, w14, uxtb`: ADDS (extended register), from reading a number in a
  POSIX `TZ` rule with `checked_add`. Nebu has ADD, SUB and SUBS in this form.
- `ldrsw x8, [sp, #...]`: LDRSW (immediate), widening a zone's offset, a signed
  32-bit number, to 64 bits. Nebu has LDRSW (register).

## Options

- **Read the numbers in NeOS a byte at a time with shifts,** which compiles to
  instructions Nebu has. That is `from_be_bytes` written out by hand, and the
  compiler may still choose `rev` for it.
- **Implement REV alone,** the one instruction seen so far.
- **Implement the family** _(chosen)_: REV16, REV32 and REV share one encoding, and
  differ only in how big a piece's bytes are reversed in. Network byte order
  (NeOS ADR 0060) will want REV16 for port numbers soon.

ADDS and LDRSW have no reasonable way round in NeOS; each is the obvious encoding of
ordinary Rust, and each completes a group Nebu half has.

## Decision

1. REV16 (both widths), REV32 (64-bit) and REV (both widths) are decoded with RBIT
   and CLZ, in data-processing (1 source), and reverse the bytes of each 16-bit,
   32-bit or whole-register piece. The 32-bit form with REV's 64-bit opcode is left
   unallocated, as the architecture has it.
2. ADDS (extended register) is decoded beside ADD, SUB and SUBS, and sets the flags
   as ADDS (shifted register) does. ADR 0041 brought the same instruction to `main`
   first, for the process model; this decision and that one are one implementation.
3. LDRSW (immediate) is decoded in its unsigned offset, pre-index and post-index
   forms, and LDURSW with the other unscaled loads.

## Consequences

- Nebu runs NeOS's `clock` scenario again, now that it checks local time.
- ADCS and the other sign-extending loads (LDRSH) are still left out, until NeOS
  needs them.
- Big-endian formats, TZif now and network headers next, cost NeOS nothing in Nebu.
