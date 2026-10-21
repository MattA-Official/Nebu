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
| 0006 | [Machine model](0006-machine-model.md)                           | Accepted; amended by 0007, 0008, 0009, 0011, 0016 and 0017 |
| 0007 | [Serial input and GPIO](0007-serial-input-and-gpio.md)           | Accepted |
| 0008 | [Exception levels and exceptions](0008-exception-levels-and-exceptions.md) | Accepted; amended by 0009, 0010 and 0016 |
| 0009 | [The MMU](0009-the-mmu.md) | Accepted |
| 0010 | [EL0 and system calls](0010-el0-and-system-calls.md) | Accepted |
| 0011 | [The firmware mailbox](0011-the-firmware-mailbox.md) | Accepted |
| 0012 | [Instructions for capabilities](0012-instructions-for-capabilities.md) | Accepted |
| 0013 | [SBC](0013-sbc.md) | Accepted |
| 0014 | [Threads and address spaces](0014-threads-and-address-spaces.md) | Accepted |
| 0015 | [LDRSW (register)](0015-ldrsw.md) | Accepted |
| 0016 | [The timer and interrupts](0016-the-timer-and-interrupts.md) | Accepted; amended by 0017 |
| 0017 | [Device interrupts and the system timer](0017-device-interrupts-and-the-system-timer.md) | Accepted |
| 0018 | [The UART's interrupts](0018-the-uarts-interrupts.md) | Proposed |
