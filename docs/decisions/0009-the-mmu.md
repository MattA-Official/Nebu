# 0009. The MMU

- **Status:** Accepted
- **Date:** 2026-10-19

## Context

NeOS now turns the MMU on and runs its kernel in the top half of the address space
([NeOS ADR 0013](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0013-the-mmu-and-the-kernel-window.md)).
Its boot code builds a temporary set of page tables, then switches to a second set
in which the kernel's code is read-only and the lower half of the address space is
switched off. Its `mmu` test writes to the kernel's own code and expects the CPU to
take a data abort with a permission fault.

ADR 0008 made Nebu halt if SCTLR_EL1.M was set, and it modelled no aborts. To run
NeOS stage 2, Nebu needs:

- the four registers that describe the page tables: TCR_EL1, MAIR_EL1, TTBR0_EL1 and
  TTBR1_EL1;
- a page table walk on every instruction fetch and data access;
- instruction and data aborts, with the syndrome QEMU reports, because the kernel's
  fault report prints ESR_EL1 and FAR_EL1;
- eleven more instructions that the new kernel code uses.

## Options

### How much of translation to model

The architecture's translation system is large: four page sizes, a range of address
sizes, two stages, hierarchical permissions, hardware-managed access flags and more.

- **All of the EL1&0 stage 1 regime:** faithful, but most of it would have no user
  and no test.
- **Exactly what NeOS uses:** 4 KiB pages, 39-bit virtual addresses in both halves,
  block and page descriptors, the access flag, AP, PXN and UXN, and MAIR's memory
  types. Any other setting halts Nebu with an explanation, as ADR 0001 asks.

### TLBs

Hardware caches translations in TLBs, and software must invalidate them after
changing the tables.

- **Model a TLB:** Nebu would then catch a missing `tlbi`, which is the kind of bug
  that is hard to find on hardware. But a TLB's contents depend on the hardware's
  replacement policy and speculation, so a model can only approximate when a stale
  entry is used. Its results would differ from QEMU's.
- **No TLB:** walk the tables on every access. A missing `tlbi` goes unnoticed, as it
  does in QEMU most of the time, but the behaviour is simple and exactly defined.

### Misaligned accesses

With the MMU on, Normal memory allows misaligned accesses, and NeOS's code may now
contain them. Device memory still faults.

## Decision

1. **Registers.** MRS and MSR support TCR_EL1, MAIR_EL1, TTBR0_EL1 and TTBR1_EL1 from
   EL1. SCTLR_EL1 accepts M (the MMU), C and I (the caches, which Nebu does not have)
   and SA and SA0 (stack alignment checks) on top of its RES1 bits. Any other
   SCTLR_EL1 bit halts Nebu. Nebu still halts on every misaligned SP-based access,
   whether or not SA is set.
2. **Translation.** With SCTLR_EL1.M set, every fetch and data access at EL1 or EL0 is
   translated. EL2 is still untranslated, because NeOS only runs there with the MMU
   off. The walk supports:
   - TCR_EL1 with T0SZ = T1SZ = 25 and 4 KiB pages in both halves, so walks start at
     level 1. Anything else halts Nebu when it is first used.
   - EPD0 and EPD1, which make every address in that half fault.
   - IPS for 32, 36 and 40-bit physical addresses, with address size faults for
     output addresses beyond them.
   - Table descriptors at levels 1 and 2, blocks at levels 1 and 2 and pages at level
     3. A table descriptor with any of its attribute bits (APTable, XNTable, PXNTable,
     NSTable) set halts Nebu.
   - The access flag, which faults if clear. Nebu never sets it itself.
   - AP\[2:1\], PXN and UXN, including the rule that EL1 cannot execute memory that
     EL0 can write.
   - MAIR_EL1's memory types: an attribute with a zero top nibble is Device memory,
     anything else Normal memory. Fetching an instruction from Device memory halts
     Nebu.

   Addresses whose top 25 bits are neither all zeros nor all ones fault at level 0, as
   in QEMU.
3. **No TLB.** Nebu walks the tables on every access. TLBI VMALLE1 is decoded and does
   nothing, like the cache maintenance instructions. Nebu therefore cannot catch a
   missing TLB invalidation.
4. **Aborts.** A failed translation takes an instruction abort (class 0x21, or 0x20
   from EL0) or a data abort (0x25, or 0x24 from EL0) to EL1. ESR_EL1 holds the fault
   status code and, for data aborts, WnR. As in QEMU, ISV is 0, so the rest of the
   syndrome is empty. FAR_EL1 gets the faulting virtual address, and ELR_EL1 the
   instruction, which has had no effect apart from any earlier part of a store pair.
5. **Misaligned accesses.** A misaligned access to Normal memory is performed a byte at
   a time. One that crosses a 4 KiB page halts Nebu. A misaligned access to Device
   memory still halts Nebu.
6. **Instructions.** ADRP, BLR, CCMP (immediate and register), CSINC, EOR (immediate),
   ANDS (immediate), ORN (shifted register), SUB (shifted register), STR (register)
   and TLBI VMALLE1.

## Consequences

- Nebu runs NeOS stage 2, including the `mmu` test, with the same transcript as QEMU.
  A deliberate write to address 0x1000 also gives the same level 0 translation fault
  in both.
- Every access now costs a three-level walk. The NeOS test suite still runs in a few
  seconds. A cache of recent translations can come later if it gets slow, as long as
  it is flushed whenever anything that affects a translation is written, so that it
  never changes what Nebu does.
- EL0's permissions are modelled, ready for NeOS's first user thread.
