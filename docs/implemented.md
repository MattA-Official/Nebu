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
| ADDS (immediate)             | NeOS step 3: chainloader     |
| ADR                          | NeOS step 2: boot            |
| AND (immediate)              | NeOS step 2: boot            |
| AND (shifted register)       | NeOS step 3: CRC-32          |
| ANDS (shifted register)      | NeOS stage 1: fault reports  |
| B                            | NeOS step 2: boot            |
| B.cond                       | NeOS step 2: boot            |
| BFM                          | NeOS step 2: boot            |
| BL                           | NeOS step 2: boot            |
| BR                           | NeOS step 3: relocation, starting the kernel |
| BRK                          | NeOS stage 1: exceptions test |
| CBNZ                         | NeOS stage 1: fault reports  |
| CBZ                          | NeOS step 2: boot            |
| CSEL                         | NeOS stage 1: fault reports  |
| DSB                          | NeOS step 3: starting the kernel |
| EOR (shifted register)       | NeOS step 3: CRC-32          |
| ERET                         | NeOS stage 1: dropping to EL1, returning from exceptions |
| HLT                          | NeOS step 2: test exit       |
| IC IALLU                     | NeOS step 3: starting the kernel |
| ISB                          | NeOS step 2: UART polling    |
| LDP                          | NeOS step 3: chainloader     |
| LDR (immediate)              | NeOS step 2: boot            |
| LDR (literal)                | NeOS step 3: relocation      |
| LDR (register)               | NeOS stage 1: fault reports  |
| LDRB (immediate)             | NeOS step 2: panic path      |
| LDRB (register)              | NeOS step 2: panic path      |
| LSLV                         | NeOS stage 1: fault reports  |
| LSRV                         | NeOS stage 1: fault reports  |
| MOVK                         | NeOS step 2: boot            |
| MOVN                         | NeOS step 3: CRC-32          |
| MOVZ                         | NeOS step 2: boot            |
| MRS                          | NeOS step 2: boot            |
| MSR (register)               | NeOS stage 1: dropping to EL1, exception vectors |
| NOP                          | NeOS step 2: boot            |
| ORR (immediate)              | NeOS stage 1: dropping to EL1 |
| ORR (shifted register)       | NeOS step 2: panic path      |
| RET                          | NeOS step 2: panic path      |
| SBFM                         | NeOS step 3: CRC-32          |
| STP                          | NeOS step 2: test exit       |
| STR (immediate)              | NeOS step 2: boot            |
| STRB (immediate)             | NeOS step 3: storing the kernel |
| STRH (immediate)             | NeOS step 2: panic path      |
| STUR                         | NeOS step 2: boot            |
| SUB (immediate)              | NeOS step 2: boot            |
| SUBS (immediate)             | NeOS step 2: panic path      |
| SUBS (shifted register)      | NeOS step 2: boot            |
| TBNZ                         | NeOS step 2: boot            |
| TBZ                          | NeOS step 2: boot            |
| UBFM                         | NeOS step 3: CRC-32          |
| UDF                          | NeOS stage 1: exceptions test |
| WFE                          | NeOS step 2: parking cores   |

Aliases are covered by the instruction they stand for: `mov` to or from SP is ADD
(immediate), `mov` between registers is ORR (shifted register), `cmp` is SUBS,
`cmn` is ADDS, `bfxil` and `bfi` are BFM, `lsl`, `lsr` and `ubfiz` are UBFM, `sbfx`
and `sxtb` are SBFM, `mov` with an immediate is MOVZ or MOVN, `tst` is ANDS, and
`lsl` and `lsr` with a register shift amount are LSLV and LSRV.

## System registers

| Register    | Access     | Notes                                       | Needed by                    |
|-------------|------------|---------------------------------------------|------------------------------|
| MPIDR_EL1   | Read       | `0x8000_0000` (core 0)                      | NeOS step 2: boot            |
| CurrentEL   | Read       | The current EL                              | NeOS step 2: boot            |
| HCR_EL2     | Read/write | Only RW may be set                          | NeOS stage 1: dropping to EL1 |
| CNTHCTL_EL2 | Read/write | Stored only: there are no timers yet        | NeOS stage 1: dropping to EL1 |
| CNTVOFF_EL2 | Read/write | Stored only                                 | NeOS stage 1: dropping to EL1 |
| SCTLR_EL1   | Read/write | The MMU (bit M) may not be turned on        | NeOS stage 1: dropping to EL1 |
| SPSR_EL2, ELR_EL2 | Read/write | Used by `eret` at EL2                 | NeOS stage 1: dropping to EL1 |
| SP_EL1      | Read/write | From EL2                                    | NeOS stage 1: dropping to EL1 |
| VBAR_EL1    | Read/write | The low 11 bits read as zero                | NeOS stage 1: exception vectors |
| SPSR_EL1, ELR_EL1 | Read/write | Set on exception entry, used by `eret` | NeOS stage 1: exception vectors |
| ESR_EL1     | Read       | Set on exception entry                      | NeOS stage 1: exception vectors |
| FAR_EL1     | Read       | Never set: only aborts set it, and Nebu has none | NeOS stage 1: exception vectors |
| SP_EL0      | Read/write |                                             | NeOS stage 1: exception vectors |

Accessing a register from a lower EL than the table allows halts Nebu, because
hardware would trap. EL1 registers need EL1 or above, and EL2 registers need EL2.

## Exceptions

| Exception                  | Class | Needed by                    |
|----------------------------|-------|------------------------------|
| Breakpoint (`brk`)         | 0x3C  | NeOS stage 1: exceptions test |
| Undefined instruction (`udf`) | 0x00 | NeOS stage 1: exceptions test |

Both are taken to EL1. Any other exception halts Nebu
([ADR 0008](decisions/0008-exception-levels-and-exceptions.md)).

## Devices and memory-mapped registers

| Device            | Registers                                         | Needed by          |
|-------------------|---------------------------------------------------|--------------------|
| RAM               | 0x0000_0000 to 0x3F00_0000                        | NeOS step 2: boot  |
| PL011 UART        | DR (write), FR (read), ICR (write); IBRD, FBRD, LCRH, CR, IMSC (read and write) | NeOS step 2: console |
| PL011 UART        | DR (read): received bytes, from standard input    | NeOS step 3: chainloader |
| GPIO              | GPFSEL1, GPPUD, GPPUDCLK0 (read and write)        | NeOS step 3: routing the UART to the pins |

## Semihosting

| Operation  | Needed by              |
|------------|------------------------|
| SYS_EXIT   | NeOS step 2: test exit |
