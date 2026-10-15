# 0000. Record architecture decisions

- **Status:** Proposed
- **Date:** 2026-10-15

## Context

Nebu is the companion emulator to NeOS (NeOS ADR 0006). The two projects should
make decisions the same way, so their records read as one history.

## Options

### Option A: A separate process for Nebu

This gives more freedom, but the two projects would drift apart.

### Option B: Adopt the NeOS process

Use the same ADR format, template and review flow.

## Decision

Adopt the NeOS process as defined in
[NeOS ADR 0000](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0000-record-architecture-decisions.md),
using [`template.md`](template.md). Cross-project decisions are recorded in NeOS
and linked from here.

## Consequences

- One way of working across both repositories.
- A Nebu ADR may depend on a NeOS ADR, and should link to it.
