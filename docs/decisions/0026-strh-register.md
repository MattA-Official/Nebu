# 0026. STRH (register)

- **Status:** Proposed
- **Date:** 2026-10-23

## Context

NeOS's keyboard stage
([NeOS ADR 0039](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0039-the-usb-keyboard.md))
adds a HID report descriptor parser, which keeps the usages a field lists in an array
of halfwords. The compiler stores into it with `strh w11, [x26, x9, lsl #1]`: STRH
with a register offset, scaled. Nebu halts on it, since it implements STRH only with
an immediate offset, and LDRH with both.

## Options

- **Change the parser so the compiler picks another instruction.** Nebu's job is to
  run whatever NeOS compiles to (ADR 0001), and any array of halfwords could bring it
  back.
- **Implement STRH (register)** _(chosen)_, sharing LDRH (register)'s decoding and
  addressing: an extended register offset, shifted by 0 or 1.

## Decision

1. STRH (register) is implemented with every extension of the offset register, and a
   shift of 0 or 1, as LDRH (register) is.

## Consequences

- Nebu runs NeOS's `keyboard` scenario.
- Nebu implements 95 instructions.
