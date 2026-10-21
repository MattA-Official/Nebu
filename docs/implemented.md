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
| ADD (extended register)      | NeOS stage 5a: threads       |
| ADD (shifted register)       | NeOS step 2: panic path      |
| ADDS (immediate)             | NeOS step 3: chainloader     |
| ADDS (shifted register)      | NeOS stage 3: loading the root task |
| ADR                          | NeOS step 2: boot            |
| ADRP                         | NeOS stage 2: MMU            |
| AND (immediate)              | NeOS step 2: boot            |
| AND (shifted register)       | NeOS step 3: CRC-32          |
| ANDS (immediate)             | NeOS stage 2: MMU            |
| ANDS (shifted register)      | NeOS stage 1: fault reports  |
| B                            | NeOS step 2: boot            |
| B.cond                       | NeOS step 2: boot            |
| BFM                          | NeOS step 2: boot            |
| BIC (shifted register)       | NeOS stage 4: capabilities   |
| BICS (shifted register)      | NeOS stage 4: capabilities   |
| BL                           | NeOS step 2: boot            |
| BLR                          | NeOS stage 2: switching page tables |
| BR                           | NeOS step 3: relocation, starting the kernel |
| BRK                          | NeOS stage 1: exceptions test |
| CBNZ                         | NeOS stage 1: fault reports  |
| CBZ                          | NeOS step 2: boot            |
| CCMN (immediate)             | NeOS stage 4: capabilities   |
| CCMP (immediate)             | NeOS stage 2: MMU            |
| CCMP (register)              | NeOS stage 2: MMU            |
| CLZ                          | NeOS stage 4: splitting memory into untyped blocks |
| CSEL                         | NeOS stage 1: fault reports  |
| CSINC                        | NeOS stage 2: MMU            |
| CSINV                        | NeOS stage 4: capability test |
| DC CIVAC                     | NeOS stage 4: asking the firmware about memory |
| DSB                          | NeOS step 3: starting the kernel |
| EOR (immediate)              | NeOS stage 2: MMU            |
| EOR (shifted register)       | NeOS step 3: CRC-32          |
| ERET                         | NeOS stage 1: dropping to EL1, returning from exceptions |
| EXTR                         | NeOS stage 4: capabilities   |
| HLT                          | NeOS step 2: test exit       |
| IC IALLU                     | NeOS step 3: starting the kernel |
| ISB                          | NeOS step 2: UART polling    |
| LDP                          | NeOS step 3: chainloader     |
| LDR (immediate)              | NeOS step 2: boot            |
| LDR (literal)                | NeOS step 3: relocation      |
| LDR (register)               | NeOS stage 1: fault reports  |
| LDRB (immediate)             | NeOS step 2: panic path      |
| LDRB (register)              | NeOS step 2: panic path      |
| LDRH (immediate)             | NeOS stage 5a: ASIDs         |
| LDRSB (register)             | NeOS stage 7: checking UTF-8 ([ADR 0018](decisions/0018-the-uarts-interrupts.md)) |
| LDRSW (register)             | NeOS stage 5b: the ipc test's jump tables |
| LDUR                         | NeOS stage 4: reading the boot info |
| LDURB                        | NeOS stage 3: loading the root task |
| LDURH                        | NeOS stage 5a: ASIDs         |
| LSLV                         | NeOS stage 1: fault reports  |
| LSRV                         | NeOS stage 1: fault reports  |
| MADD                         | NeOS stage 3: loading the root task |
| MOVK                         | NeOS step 2: boot            |
| MOVN                         | NeOS step 3: CRC-32          |
| MOVZ                         | NeOS step 2: boot            |
| MRS                          | NeOS step 2: boot            |
| MSR (register)               | NeOS stage 1: dropping to EL1, exception vectors |
| MSUB                         | NeOS stage 4: printing decimal numbers |
| NOP                          | NeOS step 2: boot            |
| ORN (shifted register)       | NeOS stage 2: MMU            |
| ORR (immediate)              | NeOS stage 1: dropping to EL1 |
| ORR (shifted register)       | NeOS step 2: panic path      |
| RBIT                         | NeOS stage 4: splitting memory into untyped blocks |
| RET                          | NeOS step 2: panic path      |
| SBC                          | NeOS stage 4: checking a split of untyped memory |
| SBCS                         | NeOS stage 4: capabilities   |
| SBFM                         | NeOS step 3: CRC-32          |
| STP                          | NeOS step 2: test exit       |
| STR (immediate)              | NeOS step 2: boot            |
| STR (register)               | NeOS stage 2: MMU            |
| STRB (immediate)             | NeOS step 3: storing the kernel |
| STRB (register)              | NeOS stage 7: the console's buffers ([ADR 0018](decisions/0018-the-uarts-interrupts.md)) |
| STRH (immediate)             | NeOS step 2: panic path      |
| STUR                         | NeOS step 2: boot            |
| STURB                        | NeOS stage 3: loading the root task |
| STURH                        | NeOS stage 5a: ASIDs         |
| SUB (immediate)              | NeOS step 2: boot            |
| SUB (shifted register)       | NeOS stage 2: MMU            |
| SUBS (extended register)     | NeOS stage 4: capability test |
| SUBS (immediate)             | NeOS step 2: panic path      |
| SUBS (shifted register)      | NeOS step 2: boot            |
| SVC                          | NeOS stage 3: system calls   |
| TBNZ                         | NeOS step 2: boot            |
| TBZ                          | NeOS step 2: boot            |
| TLBI ASIDE1                  | NeOS stage 5a: deleting an address space |
| TLBI VMALLE1                 | NeOS stage 2: switching page tables |
| UBFM                         | NeOS step 3: CRC-32          |
| UDF                          | NeOS stage 1: exceptions test |
| UMULH                        | NeOS stage 4: capabilities   |
| WFE                          | NeOS step 2: parking cores   |
| WFI                          | NeOS stage 6b: waiting for a driver's interrupt |

Aliases are covered by the instruction they stand for: `mov` to or from SP is ADD
(immediate), `mov` between registers is ORR (shifted register), `cmp` is SUBS,
`cmn` is ADDS, `bfxil` and `bfi` are BFM, `lsl`, `lsr` and `ubfiz` are UBFM, `sbfx`
and `sxtb` are SBFM, `mov` with an immediate is MOVZ or MOVN, `tst` is ANDS, `lsl`
and `lsr` with a register shift amount are LSLV and LSRV, `cset` and `cinc` are
CSINC, `mvn` is ORN, `neg` is SUB (shifted register), `mul` is MADD, `cmn` with a
register is ADDS (shifted register), `ngc` and `ngcs` are SBC and SBCS, `ror` with an immediate is EXTR,
`csetm` is CSINV, and `cmp` with an extended register is SUBS (extended register).
LDURB and STURB run as LDRB and STRB with an unscaled offset
([ADR 0010](decisions/0010-el0-and-system-calls.md)), LDUR as LDR
([ADR 0012](decisions/0012-instructions-for-capabilities.md)), and LDURH and STURH
as LDRH and STRH ([ADR 0014](decisions/0014-threads-and-address-spaces.md)).

## System registers

| Register    | Access     | Notes                                       | Needed by                    |
|-------------|------------|---------------------------------------------|------------------------------|
| MPIDR_EL1   | Read       | `0x8000_0000` (core 0)                      | NeOS step 2: boot            |
| CurrentEL   | Read       | The current EL                              | NeOS step 2: boot            |
| HCR_EL2     | Read/write | Only RW may be set                          | NeOS stage 1: dropping to EL1 |
| CNTHCTL_EL2 | Read/write | Stored only                                 | NeOS stage 1: dropping to EL1 |
| CNTVOFF_EL2 | Read/write | Subtracted from the counter to give CNTVCT_EL0 | NeOS stage 1: dropping to EL1; stage 6a: the timer |
| SCTLR_EL1   | Read/write | Only M, C, SA, SA0 and I besides the RES1 bits | NeOS stage 1: dropping to EL1; stage 2: MMU |
| SPSR_EL2, ELR_EL2 | Read/write | Used by `eret` at EL2                 | NeOS stage 1: dropping to EL1 |
| SP_EL1      | Read/write | From EL2                                    | NeOS stage 1: dropping to EL1 |
| VBAR_EL1    | Read/write | The low 11 bits read as zero                | NeOS stage 1: exception vectors |
| SPSR_EL1, ELR_EL1 | Read/write | Set on exception entry, used by `eret` | NeOS stage 1: exception vectors |
| ESR_EL1     | Read       | Set on exception entry                      | NeOS stage 1: exception vectors |
| FAR_EL1     | Read       | Set by aborts                               | NeOS stage 1: exception vectors |
| SP_EL0      | Read/write |                                             | NeOS stage 1: exception vectors |
| TCR_EL1     | Read/write | 39-bit addresses and 4 KiB pages only       | NeOS stage 2: MMU            |
| MAIR_EL1    | Read/write | Device or Normal memory, from each attribute's top nibble | NeOS stage 2: MMU |
| TTBR0_EL1, TTBR1_EL1 | Read/write | ASIDs are ignored: there is no TLB | NeOS stage 2: MMU; stage 5a: ASIDs |
| CPACR_EL1   | Read/write | Only 0 (FP and SIMD trap) may be written    | NeOS stage 3: trapping floating point |
| CTR_EL0     | Read       | `0x8444_8004`: 64-byte cache lines; EL1 and above only | NeOS stage 4: cleaning the data cache |
| TPIDR_EL0   | Read/write | From EL0 too                                | NeOS stage 5a: saving a thread's registers |
| CNTFRQ_EL0  | Read       | 62,500,000, as QEMU's `raspi3b` reports     | NeOS stage 6a: the timer     |
| CNTVCT_EL0  | Read       | One count for each instruction run          | NeOS stage 6a: the timer     |
| CNTV_CTL_EL0 | Read/write | ENABLE and IMASK; ISTATUS reads            | NeOS stage 6a: the timer     |
| CNTV_CVAL_EL0 | Read/write | The virtual timer's deadline              | NeOS stage 6a: the timer     |
| CNTKCTL_EL1 | Read/write | Only 0: EL0 cannot reach the counter or timers | NeOS stage 6a: the timer  |

Accessing a register from a lower EL than the table allows halts Nebu, because
hardware would trap. EL1 registers need EL1 or above, and EL2 registers need EL2.
TPIDR_EL0 is the one register EL0 may use. The timer's registers need EL1, as
CNTKCTL_EL1 = 0 makes them on hardware ([ADR 0016](decisions/0016-the-timer-and-interrupts.md)).

## Exceptions

| Exception                  | Class | Needed by                    |
|----------------------------|-------|------------------------------|
| Breakpoint (`brk`)         | 0x3C  | NeOS stage 1: exceptions test |
| Undefined instruction (`udf`) | 0x00 | NeOS stage 1: exceptions test |
| Instruction abort          | 0x20, 0x21 | NeOS stage 2: MMU      |
| Data abort                 | 0x24, 0x25 | NeOS stage 2: MMU test |
| Supervisor call (`svc`)    | 0x15  | NeOS stage 3: system calls |
| IRQ, from the virtual timer | none | NeOS stage 6a: preemption |
| IRQ, from the BCM2835 controller | none | NeOS stage 6b: drivers' interrupts |

All are taken to EL1. Any other exception halts Nebu
([ADRs 0008](decisions/0008-exception-levels-and-exceptions.md),
[0009](decisions/0009-the-mmu.md) and [0010](decisions/0010-el0-and-system-calls.md)).
Exceptions from EL0 use the vectors at offset 0x400. IRQs are taken between
instructions while PSTATE.I is clear, at the IRQ vectors 0x80 after the synchronous
ones, and leave ESR_EL1 as it was ([ADR 0016](decisions/0016-the-timer-and-interrupts.md)).

## Address translation

The EL1&0 stage 1 regime, with 4 KiB pages and 39-bit addresses in both halves:
block and page descriptors, the access flag, AP, PXN and UXN, and translation,
access flag, permission and address size faults. There is no TLB: every access walks
the tables ([ADR 0009](decisions/0009-the-mmu.md)).

## Devices and memory-mapped registers

| Device            | Registers                                         | Needed by          |
|-------------------|---------------------------------------------------|--------------------|
| RAM               | 0x0000_0000 to 0x3F00_0000                        | NeOS step 2: boot  |
| PL011 UART        | DR (write), FR (read), ICR (write); IBRD, FBRD, LCRH, CR, IMSC (read and write) | NeOS step 2: console |
| PL011 UART        | DR (read): received bytes, from standard input    | NeOS step 3: chainloader |
| PL011 UART        | IFLS (read and write), RIS and MIS (read); the receive and transmit interrupts, as QEMU raises them; IMSC accepts only them and the receive timeout ([ADR 0018](decisions/0018-the-uarts-interrupts.md)) | NeOS stage 7: the serial driver |
| GPIO              | GPFSEL1, GPPUD, GPPUDCLK0 (read and write)        | NeOS step 3: routing the UART to the pins |
| Mailbox           | Mailbox 0's READ and STATUS, mailbox 1's WRITE; channel 8 with the GET_ARM_MEMORY tag only ([ADR 0011](decisions/0011-the-firmware-mailbox.md)) | NeOS stage 4: how much memory there is |
| Interrupt controller | DISABLE_IRQS_1, DISABLE_IRQS_2, DISABLE_BASIC_IRQS (write) at 0x3F00_B21C to 0x3F00_B224 | NeOS stage 6a: turning off what is not used |
| Interrupt controller | IRQ_PENDING_1 and 2 (read: raised interrupts), FIQ control (write 0 only), ENABLE_IRQS_1 (write: interrupts 1 and 3 only) and ENABLE_IRQS_2 (write: interrupt 57, the UART's, only) ([ADRs 0017](decisions/0017-device-interrupts-and-the-system-timer.md) and [0018](decisions/0018-the-uarts-interrupts.md)) | NeOS stages 6b and 7: drivers' interrupts |
| Local peripherals | At 0x4000_0000: the GPU routing, the local timer's control and core 0's mailbox interrupt control (write 0 only); core 0's timer interrupt control (0 or the virtual timer); core 0's IRQ source (read: the virtual timer and the BCM2835 line) ([ADRs 0016](decisions/0016-the-timer-and-interrupts.md) and [0017](decisions/0017-device-interrupts-and-the-system-timer.md)) | NeOS stages 6a and 6b: interrupts |
| System timer      | At 0x3F00_3000: CS (read; write 1 to clear), CLO and CHI (read), C0 to C3 (read and write); 1 MHz, from the core's counter ([ADR 0017](decisions/0017-device-interrupts-and-the-system-timer.md)) | NeOS stage 6b: the interrupt test |

## Semihosting

| Operation  | Needed by              |
|------------|------------------------|
| SYS_EXIT   | NeOS step 2: test exit |
