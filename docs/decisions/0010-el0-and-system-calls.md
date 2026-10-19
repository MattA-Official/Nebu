# 0010. EL0 and system calls

- **Status:** Proposed
- **Date:** 2026-10-19

## Context

NeOS stage 3 runs its first user program, the root task, at EL0
([NeOS ADR 0014](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0014-the-first-user-thread.md)).
The kernel loads it into the lower half of the address space through TTBR0_EL1,
drops to EL0 with `eret`, and gets control back when the program executes `svc #0`
or faults. Its `user-fault` test reads the kernel's memory from EL0 and expects a
data abort from a lower EL, with a permission fault.

ADR 0008 already modelled exception levels down to EL0, `eret` to EL0 and taking
exceptions from EL0 to the "lower EL" vectors at offset 0x400, ready for this. ADR
0009 already checks EL0's page permissions and reports aborts from EL0 with their
lower-EL classes. What is missing:

- the `svc` instruction, which nothing has used until now;
- CPACR_EL1, which the kernel now clears so that floating-point and SIMD
  instructions trap;
- five instructions that the new kernel and root task code uses: MADD, ADDS (shifted
  register), LDURB, STURB and SVC itself.

## Options

### CPACR_EL1

- **Model FPEN fully:** take an exception (class 0x07) for floating-point and SIMD
  instructions when FPEN traps them. Nebu decodes none of those instructions, so
  there would be nothing to trap.
- **Accept only the value NeOS writes, 0:** everything traps. Nebu halts on those
  instructions anyway, which is how ADR 0001 asks it to treat something it does not
  model. Any other value would mean NeOS expects them to run, so Nebu halts on it.

### LDURB and STURB

They behave exactly like LDRB and STRB with an immediate offset; only the encoding
of the offset differs.

- **New instruction variants:** more code for the same behaviour.
- **Decode them to the existing variants** with an unscaled offset. Disassembly then
  prints `ldrb` and `strb` with the offset, which assemblers accept and encode as
  LDURB and STURB anyway.

## Decision

1. **SVC** takes a synchronous exception with class 0x15, IL set, and the
   instruction's 16-bit immediate as the syndrome. Its preferred return address is
   the next instruction, unlike BRK and UDF, which return to themselves. From EL0 it
   uses the lower-EL vector at offset 0x400, as ADR 0008 already does for every
   exception from EL0.
2. **CPACR_EL1** can be read and written at EL1 and above. Only 0 can be written.
   Anything else halts Nebu, because Nebu has no floating-point or SIMD instructions
   to allow.
3. **MADD**, **ADDS (shifted register)**, **LDURB** and **STURB** are implemented,
   with LDURB and STURB decoded as LDRB and STRB with an unscaled offset.
4. Accessing a system register from EL0 still halts Nebu rather than taking an
   undefined instruction exception (ADR 0008). The root task never does it.

## Consequences

- Nebu runs all seven of NeOS's test scenarios, including `user-fault`, with the
  same output as QEMU.
- Every instruction needed so far by user code is one the kernel also needed, apart
  from SVC. Later user programs will need more, as the kernel did.
- When NeOS starts using floating point in user programs, CPACR_EL1 and the
  instructions both need modelling, and a new ADR.
