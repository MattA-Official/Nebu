# What Nebu implements

Every instruction, system register and device Nebu supports, with the NeOS change
that needed it ([ADR 0001](decisions/0001-scope.md)). Anything not listed halts Nebu.

An instruction means one instruction page of the Arm Architecture Reference Manual.
Every form on a supported page is implemented, including both register widths and
all addressing modes.

## Instructions

| Instruction                  | Needed by                    |
|------------------------------|------------------------------|
| ADD (immediate)              | NeOS step 2: boot            |
| ADD (shifted register)       | NeOS step 2: panic path      |
| ADR                          | NeOS step 2: boot            |
| AND (immediate)              | NeOS step 2: boot            |
| B                            | NeOS step 2: boot            |
| B.cond                       | NeOS step 2: boot            |
| BFM                          | NeOS step 2: boot            |
| BL                           | NeOS step 2: boot            |
| CBZ                          | NeOS step 2: boot            |
| HLT                          | NeOS step 2: test exit       |
| ISB                          | NeOS step 2: UART polling    |
| LDR (immediate)              | NeOS step 2: boot            |
| LDRB (immediate)             | NeOS step 2: panic path      |
| LDRB (register)              | NeOS step 2: panic path      |
| MOVK                         | NeOS step 2: boot            |
| MOVZ                         | NeOS step 2: boot            |
| MRS                          | NeOS step 2: boot            |
| NOP                          | NeOS step 2: boot            |
| ORR (shifted register)       | NeOS step 2: panic path      |
| RET                          | NeOS step 2: panic path      |
| STP                          | NeOS step 2: test exit       |
| STR (immediate)              | NeOS step 2: boot            |
| STRH (immediate)             | NeOS step 2: panic path      |
| STUR                         | NeOS step 2: boot            |
| SUB (immediate)              | NeOS step 2: boot            |
| SUBS (immediate)             | NeOS step 2: panic path      |
| SUBS (shifted register)      | NeOS step 2: boot            |
| TBNZ                         | NeOS step 2: boot            |
| TBZ                          | NeOS step 2: boot            |
| WFE                          | NeOS step 2: parking cores   |

Aliases are covered by the instruction they stand for: `mov` to or from SP is ADD
(immediate), `mov` between registers is ORR (shifted register), `cmp` is SUBS,
`bfxil` is BFM and `mov` with an immediate is MOVZ.

## System registers

| Register    | Access | Value                       | Needed by         |
|-------------|--------|-----------------------------|-------------------|
| MPIDR_EL1   | Read   | `0x8000_0000` (core 0)      | NeOS step 2: boot |
| CurrentEL   | Read   | The current EL (always EL2) | NeOS step 2: boot |

## Devices and memory-mapped registers

| Device            | Registers                                         | Needed by          |
|-------------------|---------------------------------------------------|--------------------|
| RAM               | 0x0000_0000 to 0x3F00_0000                        | NeOS step 2: boot  |
| PL011 UART        | DR (write), FR (read), ICR (write); IBRD, FBRD, LCRH, CR, IMSC (read and write) | NeOS step 2: console |

## Semihosting

| Operation  | Needed by              |
|------------|------------------------|
| SYS_EXIT   | NeOS step 2: test exit |
