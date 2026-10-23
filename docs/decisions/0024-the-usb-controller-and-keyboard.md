# 0024. The USB controller, a hub and a keyboard

- **Status:** Proposed
- **Date:** 2026-10-23
- **Amends:** [0011](0011-the-firmware-mailbox.md), decision 3 (which tags are
  answered), and [0019](0019-the-sd-card.md), decision 5 (which interrupts the
  controller enables)

## Context

NeOS now drives the Pi's USB port from a user-space driver
([NeOS ADR 0037](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0037-usb.md)).
What it asks of the machine:

- **The firmware mailbox's "set power state" tag** (0x0002_8001), for device 3, the
  USB controller, at boot.
- **The DesignWare DWC2 controller** at 0x3F98_0000. The driver checks GSNPSID,
  waits for GRSTCTL's "AHB idle", asks for a core soft reset and two FIFO flushes,
  sets GUSBCFG, GAHBCFG (DMA, "wait for AXI writes", interrupts), HCFG, PCGCTL, the
  three FIFO sizes, GINTMSK (start of frame, the port, the channels) and HAINTMSK,
  clears GINTSTS, powers and resets the root port through HPRT0, and reads HFNUM's
  frame number.
- **Its eight host channels**, in buffer DMA mode: HCCHAR, HCSPLT (always 0 for the
  devices NeOS meets in the emulators), HCINT, HCINTMSK, HCTSIZ and HCDMA. Channel 0
  runs control transfers, which the driver polls for; the others run interrupt IN
  transfers, started at a start of frame, and raise HCINT through HAINT.
- **DMA**: each channel's buffer is a frame NeOS maps uncached, whose bus address,
  from 0xC000_0000, the kernel gives it.
- **Interrupt 9**, the controller's, through the BCM2835 controller's first enable
  register.
- **Devices**: `cargo xtask test` gives QEMU `-device usb-kbd`. QEMU 8.2 cannot put a
  full-speed keyboard straight on its DWC2 port, so it adds a hub of its own,
  0409:55aa, and plugs the keyboard, 0627:0001, into the hub's port 1. The new `usb`
  scenario claims the keyboard, reads its report descriptor, checks a stall comes
  back as one, and runs the shell's `usb` command.

**QEMU 8.2's model** (`hw/usb/hcd-dwc2.c`), the reference, keeps the registers as
plain values with a few rules: write 1 to clear GINTSTS's and HCINT's bits; reading
GRSTCTL clears the reset and flush bits, and a core soft reset does nothing else;
HPRT0's change bits and "enabled" clear when 1 is written, and the end of a reset
enables the port and resets the device. Enabling a channel runs its transfer at once,
a packet at a time, straight to or from RAM. A NAK on a control transfer is tried
again for ever; one on an interrupt transfer halts the channel. A start of frame is
raised every millisecond from the moment a device is attached. It ignores HCSPLT and
the data toggles, and a transfer to an address no device has is never answered.

## Options

### How the controller is modelled

- **As the databook describes it**, with FIFOs, slave mode, the frame scheduler and
  real timings. Most of it NeOS never uses (ADR 0001), and the timings would make
  Nebu's runs differ from QEMU's for no gain.
- **As QEMU models it, for what NeOS uses, and halting on the rest** _(chosen)_. The
  registers change as QEMU's do, and transfers finish the moment they start. Nebu
  halts on what QEMU would silently get wrong: slave mode (no DMA), a split
  transaction, a low-speed device, bulk and isochronous transfers, a transfer no
  device answers, DMA outside RAM's uncached alias, and a register or bit NeOS does
  not use.

### Which devices, and how they are attached

- **A keyboard straight on the root port.** Simpler, but NeOS's output would differ
  from QEMU's, which always puts its hub in between, and NeOS's hub code would go
  untested in Nebu.
- **QEMU's hub with QEMU's keyboard on port 1** _(chosen)_, with the same
  descriptors byte for byte, so the `usb` command prints the same in both. A
  `--keyboard` flag plugs them in, as `-device usb-kbd` does in QEMU; without it the
  port is empty, as in QEMU.
- **A general device tree from the command line.** No test needs it yet.

### Which requests the devices answer

- **Every standard, hub and HID request**, as QEMU's do. Most NeOS never sends.
- **The requests NeOS sends, and a stall for a HID class request HID does not
  define** _(chosen)_. Any other request halts Nebu, so it learns the moment NeOS
  starts using one. The stall is what QEMU's keyboard and real ones answer, and is
  how NeOS's test checks that a stall reaches the driver's client.

## Decision

1. **The mailbox** answers "set power state" as QEMU does: it keeps the device
   number, clears every bit of the state but "on", and reports the device as being in
   that state.
2. **The DWC2 controller** is at 0x3F98_0000. Its registers start as QEMU 8.2 resets
   them, GSNPSID reads 0x4F54_294A, and HPRT0 is powered, with a full-speed device
   connected and its connection detected if `--keyboard` was given. GAHBCFG takes
   only interrupts, DMA and "wait for AXI writes"; GINTMSK only start of frame, the
   port and the channels; GRSTCTL only the core reset and the flushes, which finish
   at once and change nothing else; PCGCTL only 0; HPRT0 neither suspend, resume nor
   the test modes. Writing anything else halts Nebu.
3. **HPRT0** changes as in QEMU: its read-only bits stay, writing 1 clears its change
   bits and "enabled", "enabled" cannot be set by writing, and clearing the reset bit
   while a device is connected enables the port, sets "enabling changed" and resets
   the device. GINTSTS's port bit is raised while any change bit is set.
4. **A channel's transfer** runs when HCCHAR's enable bit is set: a SETUP packet for
   a control endpoint with PID SETUP, otherwise IN or OUT by HCCHAR's direction. It
   moves packets of up to the largest packet size, from or to RAM at HCDMA less
   0xC000_0000, until the packet count or size runs out, a packet is empty, or an IN
   packet is short. HCTSIZ and HCDMA then hold what is left, as in QEMU, and HCINT
   says "transfer complete" and "halted", or "NAK" and "halted" for an interrupt
   transfer the device has nothing for, or "stall" and "halted". Setting the disable
   bit halts a channel at once. HAINT has a bit for each channel whose HCINT and
   HCINTMSK share one, and GINTSTS's channel bit is raised while HAINT and HAINTMSK
   do.
5. **Frames**: while a device is attached, a start of frame is raised every 1 ms of
   the core's counter (62 500 counts, ADR 0016), and HFNUM counts frames from 0x3FFF,
   as QEMU's does. The next frame is a timed interrupt, like the system timer's, so a
   core waiting in `wfi` wakes for it.
6. **The interrupt**, 9 on the BCM2835 controller, is raised while GAHBCFG enables
   interrupts and GINTSTS and GINTMSK share a bit. The BCM2835 controller now also
   enables it.
7. **The devices** answer through QEMU's three stages: a setup packet, data, and a
   status packet, at which a request with no data for the host takes effect, so a
   device takes its new address only once SET_ADDRESS is over. They answer only at
   their address, the hub only while the root port is enabled, and the keyboard only
   while the hub's port is. A port reset makes a device forget its address and
   configuration. Interrupt transfers need the device configured.
8. **The hub**, 0409:55aa with eight ports and no power switching, answers its
   device, configuration and hub descriptors, Get Port Status, Set Port Feature
   (reset, and power, which does nothing), and Clear Port Feature (enable and the
   five change bits). A port reset sets "reset changed", "enabled" and "enabling
   changed", as in QEMU. Its status endpoint, 0x81, sends a bit for each port with a
   change, from bit 1, in two bytes, and NAKs when there is none.
9. **The keyboard**, 0627:0001, a full-speed boot keyboard, answers its device,
   configuration and 63-byte report descriptors, Get and Set Protocol, Get and Set
   Idle, and Set Report, which sets its LEDs. A HID class request with a number HID
   does not define stalls. Its report endpoint, 0x81, NAKs: nothing types on it yet.

## Consequences

- NeOS's USB driver, its hub code and its keyboard claim run in Nebu, and the `usb`
  scenario's output matches QEMU's byte for byte.
- A start of frame every millisecond wakes NeOS's driver while it polls the hub, so a
  test with `--keyboard` runs more instructions while it waits.
- Splits, the Pi's LAN9514 high-speed hub, low-speed keyboards and the caches are
  tested only on the Pi.
- Typing on the keyboard needs a way to give Nebu key presses, which comes with
  NeOS's keyboard stage.
