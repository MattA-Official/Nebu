# 0041. ADDS (extended register)

- **Status:** Accepted
- **Date:** 2026-11-05

## Context

NeOS's file system server learns from the kernel when a client's last capability
goes, and closes what it left open
([NeOS ADR 0078](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0078-notices-when-the-last-capability-goes.md)).
NeOS's process test checks it with a child program that leaves a folder open. The
compiler turns an addition in that program into `adds x9, x9, w10, sxtw`: ADDS with
an extended register. Nebu halts on it, since it implements ADD, SUB and SUBS with
an extended register, but not ADDS.

## Options

- **Change the program so the compiler picks another instruction.** Nebu's job is to
  run whatever NeOS compiles to (ADR 0001), and any such addition could bring it back.
- **Implement ADDS (extended register)** _(chosen)_, decoded with the other three, in
  both widths and with every extension.

## Decision

1. ADDS (extended register) is implemented in both widths: `rd` is `rn` plus `rm`
   extended and shifted, as for ADD, and the flags are set as for ADDS (shifted
   register).

## Consequences

- Nebu runs NeOS's `process` scenario with the file server's new check.
- Nebu implements all four forms of add and subtract with an extended register.
