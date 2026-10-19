# 0014. Threads and address spaces

- **Status:** Proposed
- **Date:** 2026-10-19

## Context

NeOS stage 5a makes threads and address spaces kernel objects
([NeOS ADR 0018](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0018-threads-and-address-spaces.md),
proposed in NeOS #12). The kernel now:

- saves and restores each thread's TPIDR_EL0, the thread pointer EL0 can read and
  write;
- gives every address space an 8-bit ASID, puts it in bits 63 to 48 of TTBR0_EL1,
  and marks user pages not global (nG);
- flushes an address space's translations with `tlbi aside1` when it is deleted;
- keeps 16-bit ASIDs in its records, which the compiler reads and writes with LDRH,
  LDURH and STURH, and adds with ADD (extended register).

Nebu stops on all of these today.

## Options

### ASIDs and the nG bit

Nebu has no TLB: it walks the page tables on every access (ADR 0009). ASIDs and the
nG bit only say which cached translations belong to which address space.

- **Model a TLB, tagged with ASIDs.** Nebu could then catch a kernel that forgets
  to flush, which hardware punishes with stale translations. But Nebu would have to
  choose when entries are evicted, which hardware leaves unspecified, so it could
  not match the Pi, and a bug might show in one and not the other. It is also the
  largest piece of Nebu yet, for a check QEMU and the Pi already make in their own
  ways.
- **Ignore them** _(chosen)_. Without a TLB, every walk uses the current tables, so
  the ASID changes nothing about what an access finds. TTBR0's ASID bits are kept,
  so reading the register back returns them, and the walk uses only the address
  bits, as it does now. `tlbi aside1` has nothing to flush, like `tlbi vmalle1`.

### TPIDR_EL0

- **Halt on it**, and ask NeOS not to save it. That would leave threads sharing one
  thread pointer, which a later runtime will use.
- **Store it, and let EL0 and EL1 read and write it** _(chosen)_, as hardware does.
  Nothing in Nebu uses its value.

### The new instructions

As in ADR 0012, Nebu implements exactly the instructions NeOS runs, so that each
is checked against QEMU by NeOS's tests. LDURH and STURH are LDRH and STRH with an
unscaled offset, so they share those variants, as LDURB and LDUR do.

## Decision

1. **TTBR0_EL1's ASID is stored and ignored**, and so is the nG bit. **TLBI ASIDE1**
   runs and does nothing.
2. **TPIDR_EL0** is stored, and EL0 may read and write it. It is the first register
   EL0 may access.
3. **LDRH (immediate)**, in all three indexing forms, **LDURH**, **STURH** and **ADD
   (extended register)**, in both widths with every extension, are implemented.
   ADDS and SUB (extended register), and the sign-extending loads, still halt Nebu.

## Consequences

- Nebu runs all nine of NeOS's test scenarios, including the new `threads`
  scenario, with the same output as QEMU.
- A kernel that forgets to flush an address space's translations still works in
  Nebu. QEMU, and in the end the Pi, are where such a bug would show.
- Nebu implements 88 instructions and 22 system registers.
