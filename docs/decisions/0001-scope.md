# 0001. Scope: emulate only what NeOS uses

- **Status:** Proposed
- **Date:** 2026-10-15

## Context

[NeOS ADR 0006](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0006-lockstep-emulator-nebu.md)
establishes Nebu as a minimal emulator developed in lockstep with NeOS. This ADR
defines what "minimal" means for Nebu itself.

The real machine is a Raspberry Pi 3B: a Broadcom BCM2837 with four Cortex-A53
cores (ARMv8-A), 1 GB of RAM and Broadcom peripherals. QEMU's `raspi3b` machine is
the reference implementation.

## Options

### Option A: Implement by specification

Work through the Arm Architecture Reference Manual and the BCM2837 peripheral
documentation section by section. This is thorough, but most of the work would serve
nothing NeOS runs.

### Option B: Implement on demand

Start with nothing. Run NeOS, and when Nebu hits something it does not support,
implement exactly that, checked against the specification and QEMU.

## Decision

Adopt **Option B**, with these rules:

1. **AArch64 only.** Nebu runs A64 code and has no AArch32 support. NeOS has no
   AArch32 code.
2. **Fail loudly.** Every unimplemented instruction encoding, system register,
   memory-mapped register or device access halts Nebu with an error naming the
   item, the program counter and the raw value. Nothing is ignored or guessed.
3. **Implement fully, not partially.** When an instruction is added, all of its
   behaviour is implemented, including flags, edge cases and exceptions it can
   raise, as defined in the Arm ARM. Unused *instructions* are left out, but
   *behaviours* of supported ones are never skipped.
4. **Match QEMU's starting conditions.** Nebu loads `kernel8.img` and starts the
   CPU in the same state QEMU's `raspi3b` does when given `-kernel`, including
   the exception level, load address and the state of the secondary cores. The
   exact details are recorded when they are implemented in NeOS step 2.
5. **An inventory.** `docs/implemented.md` lists every supported instruction,
   register and device, with the NeOS change that needed it.

## Consequences

- Nebu is small and every line of it is exercised by NeOS.
- Running anything other than NeOS on Nebu will usually fail quickly, which is
  intended.
- The fail-loudly rule turns every gap into a clear, actionable error message.
