# Architecture Decision Records

Nebu follows the same decision process as NeOS: see
[NeOS ADR 0000](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0000-record-architecture-decisions.md).
In short, every significant decision gets a short, numbered record that lists the
options considered. It is proposed in a pull request, accepted on approval and never
rewritten afterwards, only superseded.

Decisions that affect both projects live in NeOS. Decisions about how Nebu itself
works live here.

## Index

| No.  | Title                                                            | Status   |
|------|------------------------------------------------------------------|----------|
| 0000 | [Record architecture decisions](0000-record-architecture-decisions.md) | Accepted |
| 0001 | [Scope: emulate only what NeOS uses](0001-scope.md)              | Accepted |
| 0002 | [No dependencies, no unsafe](0002-no-dependencies-no-unsafe.md)  | Accepted |
| 0003 | [Toolchain](0003-toolchain.md)                                   | Accepted |
| 0004 | [Execution model: an interpreter](0004-execution-model.md)      | Accepted |
| 0005 | [Licence](0005-licence.md)                                       | Accepted |
| 0006 | [Machine model](0006-machine-model.md)                           | Accepted; amended by 0007, 0008, 0009, 0011, 0016, 0017 and 0030 |
| 0007 | [Serial input and GPIO](0007-serial-input-and-gpio.md)           | Accepted; amended by 0018 |
| 0008 | [Exception levels and exceptions](0008-exception-levels-and-exceptions.md) | Accepted; amended by 0009, 0010 and 0016 |
| 0009 | [The MMU](0009-the-mmu.md) | Accepted |
| 0010 | [EL0 and system calls](0010-el0-and-system-calls.md) | Accepted |
| 0011 | [The firmware mailbox](0011-the-firmware-mailbox.md) | Accepted; amended by 0023, 0024 and 0033 |
| 0012 | [Instructions for capabilities](0012-instructions-for-capabilities.md) | Accepted |
| 0013 | [SBC](0013-sbc.md) | Accepted |
| 0014 | [Threads and address spaces](0014-threads-and-address-spaces.md) | Accepted |
| 0015 | [LDRSW (register)](0015-ldrsw.md) | Accepted |
| 0016 | [The timer and interrupts](0016-the-timer-and-interrupts.md) | Accepted; amended by 0017, 0019, 0030 and 0031 |
| 0017 | [Device interrupts and the system timer](0017-device-interrupts-and-the-system-timer.md) | Accepted; amended by 0018 and 0030 |
| 0018 | [The UART's interrupts](0018-the-uarts-interrupts.md) | Accepted; amended by 0019 |
| 0019 | [The SD card](0019-the-sd-card.md) | Accepted; amended by 0024 and 0028 |
| 0020 | [LDRSB (immediate)](0020-ldrsb-immediate.md) | Accepted |
| 0021 | [Widening multiplies](0021-widening-multiplies.md) | Accepted |
| 0022 | [Loading a second file into memory](0022-loading-a-second-file.md) | Accepted |
| 0023 | [The framebuffer](0023-the-framebuffer.md) | Accepted |
| 0024 | [The USB controller, a hub and a keyboard](0024-the-usb-controller-and-keyboard.md) | Accepted; amended by 0033 |
| 0025 | [Typing on the keyboard](0025-typing-on-the-keyboard.md) | Accepted; amended by 0027 |
| 0026 | [STRH (register)](0026-strh-register.md) | Accepted |
| 0027 | [Holding keys](0027-holding-keys.md) | Accepted |
| 0028 | [Writing to the SD card](0028-writing-to-the-sd-card.md) | Accepted |
| 0029 | [CSNEG](0029-csneg.md) | Accepted |
| 0030 | [Four cores](0030-four-cores.md) | Accepted; amended by 0031 |
| 0031 | [Exclusive loads and stores, and every core's timer](0031-exclusives-and-every-cores-timer.md) | Accepted; amended by 0050 |
| 0032 | [The hardware random number generator](0032-the-random-number-generator.md) | Accepted |
| 0033 | [A network card, bulk transfers and the board's MAC address](0033-the-network-card-and-bulk-transfers.md) | Accepted |
| 0036 | [The watchdog, and restarting the machine](0036-the-watchdog-and-restarting.md) | Accepted |
| 0041 | [ADDS (extended register)](0041-adds-extended-register.md) | Accepted |
| 0042 | [DMB, ADC and SMULH](0042-dmb-adc-and-smulh.md) | Proposed |
| 0043 | [REV, ADDS (extended register) and LDRSW (immediate)](0043-rev-adds-and-ldrsw.md) | Accepted |
| 0044 | [LDRSH (immediate)](0044-ldrsh-immediate.md) | Accepted |
| 0050 | [The mailboxes between cores](0050-the-mailboxes-between-cores.md) | Accepted |
