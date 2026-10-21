# 0019. The SD card

- **Status:** Proposed
- **Date:** 2026-10-21
- **Amends:** [0016](0016-the-timer-and-interrupts.md) (CNTKCTL_EL1 only 0) and
  [0018](0018-the-uarts-interrupts.md) (decision 3, which interrupts the controller
  enables)

## Context

NeOS reads the Pi's SD card from a user-space driver
([NeOS ADR 0028](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0028-the-sd-card-driver-and-block-protocol.md)).
What it asks of the machine:

- **The GPIO.** The driver sets pins 34 to 39 to inputs and pins 48 to 53 to
  alternate function 3, through GPFSEL3, GPFSEL4 and GPFSEL5, reading each first,
  and gives pins 49 to 53 pull-ups through GPPUD and GPPUDCLK1. On the Pi this
  connects the card's slot to the Arasan EMMC controller.
- **The Arasan EMMC controller** at 0x3F30_0000. The driver resets it through
  CONTROL1, sets its clock there, chooses which interrupts it records (IRPT_MASK)
  and raises (IRPT_EN), and sends commands by writing ARG1 and then CMDTM. It reads
  the responses from RESP0 to RESP3, waits on STATUS's inhibit bits, reads each
  block's 128 words from DATA, and clears INTERRUPT's bits by writing 1s. It uses
  no DMA, and never writes to the card.
- **The card**, through CMD0 (go idle), CMD8 (interface condition), CMD55 and
  ACMD41 (operating conditions), CMD2 (identification), CMD3 (relative address),
  CMD9 (card-specific data, for its size), CMD7 (select, with a busy response) and
  CMD17 (read one block). It reads only high-capacity cards, larger than 2 GB.
- **Interrupt 62**, the controller's, through the BCM2835 controller's second
  enable register.
- **The virtual counter from EL0**: the kernel sets CNTKCTL_EL1.EL0VCTEN, and the
  driver reads CNTVCT_EL0 and CNTFRQ_EL0 to time short delays.
- **UDIV**, which the compiler uses for the driver's clock divisor and the
  runtime's conversion from microseconds to counts.

`cargo xtask test` gives QEMU a 4 GiB card image with `-drive if=sd`. The new
`sdcard` scenario, and every boot, print the card's size and its first partition.

**QEMU 8.2's SD host controller**, the reference, runs a command as soon as CMDTM
is written, with no delay, and fills its buffer with the block's data at the same
moment. It records an interrupt only if IRPT_MASK has its bit, and raises its line
while INTERRUPT and IRPT_EN share a bit. Its card powers up at the first ACMD41
that names a voltage window. A command no card answers sets the command timeout
error. Its card is attached to the Arasan controller unless all six pins choose
SDHOST (alternate function 0), so it answers whether or not NeOS sets alternate
function 3.

## Options

### How the card's image is read

- **Load the whole image into memory.** Simple, but the test image is 4 GiB, as it
  must be for QEMU and Nebu to treat it as high capacity.
- **Read each block from the file when it is asked for** _(chosen)_. The test image
  is sparse, and only a few blocks are ever read. The file is opened read-only, so
  nothing NeOS does can change it.

### When a command finishes

- **After the time it takes on a bus at the card's clock.** It would test the
  driver's waiting, but neither QEMU nor the driver's output depends on it, and the
  two emulators' runs would differ for no gain.
- **At once, as QEMU does** _(chosen)_. The interrupt is raised before the next
  instruction runs. The driver must already handle that, since it checks the
  status before it waits.

### What the pins do

- **Answer whatever the GPIO says, as QEMU does.** A NeOS that forgot to switch the
  pins would pass in both emulators and fail only on the Pi.
- **Answer only while pins 48 to 53 are all alternate function 3** _(chosen)_. A
  command sent otherwise times out, as with no card. Nebu's run then differs from
  QEMU's only when NeOS is wrong for the Pi, which is what Nebu is for (ADR 0001).

### What is modelled of the card

- **The whole SD protocol**, with every state and command. Most of it NeOS never
  uses (ADR 0001).
- **Only the commands above, in the states NeOS sends them in** _(chosen)_. Any
  other command, or one in another state, halts Nebu. A write command, or a
  transfer of anything but one 512-byte block, halts it too. Only images larger
  than 2 GiB are accepted, since NeOS refuses smaller cards, and at most 32 GiB,
  the most an SDHC card holds. The card-specific data is the version 2.0 layout
  high-capacity cards use, with the size QEMU would give the same image, so the
  image must be a whole number of its unit, 512 KiB.

## Decision

1. **`--sd <file>`** puts a high-capacity card holding `<file>` in the slot. The
   file must be a whole number of 512 KiB, larger than 2 GiB and at most 32 GiB,
   and is opened read-only. Without it the slot is empty, and every command with a
   response times out.
2. **The EMMC controller** at 0x3F30_0000 implements BLKSIZECNT, ARG1, CONTROL1,
   INTERRUPT, IRPT_MASK and IRPT_EN (read and write), CMDTM (write), and RESP0 to
   RESP3, DATA and STATUS (read), as QEMU 8.2 behaves, with commands that finish at
   once. CONTROL1's resets finish at once, and resetting the command or data line
   clears its interrupts, as QEMU's does. Its clock is stable as soon as it is
   enabled. A command with the clock off, while a block is being read, or with a
   command type, multi-block transfer, block count or automatic command, halts
   Nebu. Any other register halts it.
3. **The card** answers CMD0, CMD8, CMD55, ACMD41, CMD2, CMD3, CMD9, CMD7 and CMD17
   in the states the SD specification allows, as QEMU's card does, and only while
   pins 48 to 53 are all alternate function 3. Anything else halts Nebu.
4. **The GPIO** implements GPFSEL3, GPFSEL4, GPFSEL5 and GPPUDCLK1 (read and
   write). Pulls have no effect, since Nebu has no pins.
5. **The BCM2835 controller** also enables interrupt 62, the EMMC controller's.
6. **CNTKCTL_EL1** accepts EL0VCTEN (bit 1) as well as 0. While it is set, EL0 may
   read CNTVCT_EL0 and CNTFRQ_EL0. Any other bit still halts Nebu. This amends ADR
   0016.
7. **UDIV** is implemented, for 32-bit and 64-bit registers, with division by zero
   giving zero, as the architecture says.

## Consequences

- Nebu runs all fourteen of NeOS's test scenarios, including `sdcard`, with the
  same output as QEMU.
- Nebu catches a NeOS that does not switch the card's pins, which QEMU does not.
- The driver's waiting is exercised only for interrupts that are already raised; how
  long a real card takes is tested only on the Pi, as in QEMU.
- Writing to the card halts Nebu. A NeOS ADR that adds writing will need a Nebu ADR
  too.
- Nebu implements 93 instructions, 27 system registers, six kinds of exception and
  seven devices.
