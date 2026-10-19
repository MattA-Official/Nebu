# 0008. Exception levels and exceptions

- **Status:** Accepted
- **Date:** 2026-10-19

## Context

NeOS now runs its kernel at EL1 and handles exceptions there
([NeOS ADR 0012](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0012-exceptions-at-el1.md)).
Its boot code drops from EL2 to EL1 with `eret`, and its kernel installs a vector
table. The `exceptions` test then executes `brk` and `udf` and expects the CPU to take
an exception to EL1 each time. ADR 0006 left exceptions out entirely: anything that
would raise one halted Nebu.

To run this, Nebu needs:

- more than one exception level, each with its own stack pointer;
- the system registers the boot code and the vector table use;
- `eret`;
- exception entry for the two exceptions NeOS raises on purpose.

The kernel's fault report prints ESR_EL1 and ELR_EL1, so Nebu must set them exactly
as QEMU does, or the transcripts would differ.

## Options

### How much of exception entry to model

- **Only the exceptions NeOS raises on purpose:** `brk` and `udf`. Anything else that
  would raise an exception, such as a misaligned access or a system register access
  that would trap, still halts Nebu with an explanation.
- **Every synchronous exception:** data aborts, alignment faults, traps on system
  register access. This is faithful, but each needs its own syndrome encoding, and
  NeOS has no use for any of them yet. A halt is also easier to debug than an
  exception that NeOS reports.

### System registers

- **Store every value as written:** simple, but a write that changes behaviour, such
  as turning on the MMU, would silently do nothing.
- **Store values, but halt on any setting Nebu does not model:** an MMU, HCR_EL2
  traps or AArch32 at EL1. The halt says what was asked for.

## Decision

1. **PSTATE.** Nebu models the exception level (EL0 to EL2), PSTATE.SP (whether the
   current level uses its own stack pointer or SP_EL0), the DAIF mask bits and the
   condition flags. There are separate stack pointers SP_EL0, SP_EL1 and SP_EL2. The
   core still starts at EL2h with every exception masked.
2. **System registers.** MRS and MSR support SCTLR_EL1, SPSR_EL1, ELR_EL1, SP_EL0,
   ESR_EL1, FAR_EL1 and VBAR_EL1 from EL1 and above, and HCR_EL2, CNTHCTL_EL2,
   CNTVOFF_EL2, SPSR_EL2, ELR_EL2 and SP_EL1 from EL2. Accessing a register from too
   low a level halts Nebu, because hardware would trap. So do writes Nebu cannot
   honour: SCTLR_EL1.M (the MMU) and any HCR_EL2 bit other than RW. The timer
   registers are only stored, because Nebu has no timers yet.
3. **Exception entry.** `brk #imm` and `udf #imm` take a synchronous exception to EL1:
   - SPSR_EL1 gets PSTATE;
   - ELR_EL1 gets the address of the instruction;
   - ESR_EL1 gets the class (0x3C for `brk`, 0 for `udf`), the IL bit and, for `brk`,
     the immediate;
   - PSTATE becomes EL1h with everything masked, and execution continues at VBAR_EL1
     plus the offset for where the exception came from.

   FAR_EL1 is left alone, because the architecture leaves it UNKNOWN for these
   classes. An exception at EL2 halts Nebu, because nothing uses EL2's vectors.
4. **`eret`.** It restores PSTATE from the current level's SPSR and jumps to its ELR.
   Returns that the architecture calls illegal, such as to a higher level, to
   AArch32, or to EL1 when HCR_EL2.RW is 0, halt Nebu instead of setting PSTATE.IL.
5. **Undefined encodings.** `udf` is the only encoding that raises an Undefined
   Instruction exception. Other encodings Nebu cannot decode still halt it, because
   Nebu cannot tell an unallocated encoding from one it has not implemented.

## Consequences

- Nebu runs the NeOS `exceptions` test and produces the same transcript as QEMU,
  including the ESR and ELR values in the fault report.
- Every other exception still halts Nebu with a reason, which is the right default
  until NeOS handles it on purpose. System calls, data aborts and interrupts will
  each come with the NeOS change that needs them.
- Taking an exception from EL0 is already modelled, ready for NeOS's first user
  thread, but nothing exercises it yet.
