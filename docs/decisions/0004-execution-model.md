# 0004. Execution model: an interpreter

- **Status:** Proposed
- **Date:** 2026-10-15

## Context

An emulator has to execute guest instructions somehow. The main approaches differ
greatly in complexity, speed and how easy they are to inspect.

Lockstep testing against QEMU
([NeOS ADR 0006](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0006-lockstep-emulator-nebu.md))
needs Nebu to stop after *any* single instruction and expose its full register state.

## Options

### Option A: A fetch, decode and execute interpreter

Fetch one instruction, decode it into a Rust `enum`, then execute it with a `match`.
This is the simplest approach, and every instruction is a natural stopping point.
It is the slowest of the three, roughly 50 to 200 million instructions per second
on a modern host, which is ample for a small kernel.

### Option B: An interpreter with a decoded-block cache

The same as A, but decoded instructions are cached per block so that hot loops
are not decoded again. It is faster, but self-modifying code and page-table
changes complicate cache invalidation.

### Option C: Dynamic binary translation (a JIT), as QEMU uses

Translate guest blocks into host machine code. This is the fastest, but it is
far more complex, harder to single-step, and requires `unsafe` code to execute
generated machine code, which ADR 0002 forbids.

## Decision

Adopt **Option A**. The decoder and executor are kept separate. The decoder turns
a 32-bit word into a typed instruction and the executor applies it to the CPU
state, so each can be tested on its own. Every decoded instruction can be
printed back in standard Arm assembly syntax for traces.

## Consequences

- Single-stepping and instruction tracing come for free.
- Unit tests can check the decoder against known encodings without running anything.
- If performance ever becomes a problem, Option B can be added later without
  changing the decoder or executor, through a new ADR.
