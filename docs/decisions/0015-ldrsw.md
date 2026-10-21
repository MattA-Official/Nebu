# 0015. LDRSW (register)

- **Status:** Accepted
- **Date:** 2026-10-21

## Context

NeOS stage 5b adds IPC, notifications and fault messages
([NeOS ADR 0020](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0020-ipc-notifications-and-faults.md)).
The kernel needs nothing new from Nebu: its IPC is copies between words of object
memory, which use instructions Nebu already runs. But the new `ipc` test's root
task has a `match` that the compiler turns into a jump table of 32-bit offsets, read
with `ldrsw x12, [x10, x0, lsl #2]`. Nebu halts on it: ADR 0014 left the
sign-extending loads to halt until something ran them.

## Options

- **Change the test so the compiler picks another instruction.** Nebu's job is to run
  whatever NeOS compiles to (ADR 0001), and any later `match` could bring it back.
- **Implement every sign-extending load** (LDRSB, LDRSH and LDRSW, in every addressing
  form) at once. They share most of their decoding, but none of the others is run by
  anything, so none would be checked against QEMU.
- **Implement LDRSW (register)** _(chosen)_, as ADR 0012 does for each instruction
  NeOS runs. It is LDR (register) with a 32-bit access, sign-extended to 64 bits.

## Decision

1. LDRSW (register) is implemented, with every extension of the offset register and
   the optional shift by 2.
2. The other sign-extending loads, and LDRSW with an immediate offset, still halt
   Nebu.

## Consequences

- Nebu runs all ten of NeOS's test scenarios, including the new `ipc` scenario, with
  the same output as QEMU.
- Nebu implements 89 instructions.
