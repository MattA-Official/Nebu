# 0033. A network card, bulk transfers and the board's MAC address

- **Status:** Proposed
- **Date:** 2026-11-05
- **Amends:** [0011](0011-the-firmware-mailbox.md), decision 3 (which tags are
  answered), and [0024](0024-the-usb-controller-and-keyboard.md) (bulk transfers,
  and which device is behind the hub)

## Context

NeOS's first networking stage gives it two network card drivers, both clients of
its USB driver
([NeOS ADR 0061](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0061-the-network-drivers.md)).
What it now asks of the machine:

- **The firmware mailbox's "get board MAC address" tag** (0x0001_0003), at every
  boot. QEMU 8.2's `raspi3b` answers it with six bytes, 52:54:00:12:34:56, the first
  of QEMU's default addresses. Nebu halts on tags it does not answer, so without it
  no NeOS kernel runs in Nebu at all.
- **Bulk transfers** on the DWC2's channels, besides control and interrupt ones. A
  Bulk Out runs on channel 0, which the driver polls; a Bulk In waits on a channel of
  its own, and raises its interrupt once data comes. QEMU's model
  (`hw/usb/hcd-dwc2.c`) answers a device's NAK to a bulk or control packet by leaving
  the channel enabled, setting HCINT's NAK bit, and trying again every 250 µs; the
  channel halts only once the transfer completes or fails, with the NAK bit still
  set. A disabled channel halts at once.
- **QEMU's `usb-net`** (`hw/usb/dev-network.c`), which `cargo xtask test`'s new
  `network` scenario plugs in with `-device usb-net,netdev=net0,mac=52:54:00:12:34:57`
  on `-netdev user,id=net0,ipv6=off,restrict=on`. Like the keyboard, it goes behind a
  hub QEMU adds, on the hub's port 1. It is NetChip's RNDIS gadget, 0525:a4a2, full
  speed, with two configurations: RNDIS first (value 2), then CDC Ethernet (value 1).
  NeOS chooses CDC Ethernet, reads the MAC address from string 3, which QEMU makes
  from the address it is given with 0x40 for the first byte, in lowercase hex; turns
  the data interface's setting 1 on; and sets the packet filter. Frames go out in
  bulk transfers to endpoint 2: QEMU sends one to the network when a packet is short
  or empty. They come in from endpoint 2 one at a time, ending with a short packet,
  or with an empty one after a frame that fills its last packet; QEMU NAKs while it
  has none, and takes the next from the network only once the last has gone.
- **QEMU's user networking, slirp**, behind the card: with `restrict=on` and no
  IPv6, it sends nothing of its own. NeOS's test sends one ARP request, for the
  gateway, 10.0.2.2, from 10.0.2.15. slirp answers a request for its gateway or its
  name server (10.0.2.3) with a 64-byte frame, padded with zeros, from
  52:55:0a:00:02:02 (52:55, then the address asked for), to the request's sender.

## Options

### When a NAKed bulk transfer is tried again

- **Every 250 µs, as QEMU does**, from `advance`. The closest copy, but `advance` would
  need RAM, for the DMA, and a way to report a transfer Nebu cannot model from the
  middle of the timer's work, for retries that, with Nebu's devices, find nothing new.
- **After every other transfer** _(chosen)_. Nebu's only bulk device is the network
  card, and its network sends a frame only in answer to one sent, which reaches the
  card through a transfer. So the moment another transfer finishes is the only time
  a waiting one can have anything to take. The output is the same; only when the
  data arrives, by a few hundred microseconds, differs.

### The network behind the card

- **A copy of slirp**: IP, TCP, DHCP, DNS and the rest. Far beyond what NeOS sends
  now, and against Nebu's rule of modelling only what NeOS uses.
- **Only what NeOS's test sends** _(chosen)_: ARP requests, answered for the gateway
  and the name server as slirp answers them, and other ARP ignored, as slirp ignores
  it. Any other frame halts Nebu, so that the day NeOS sends IPv4, Nebu says so
  rather than dropping it. Later stages of NeOS's networking will grow it, an ADR at
  a time.

### Which configuration Nebu's card models

- **Both**, as QEMU's does. RNDIS is a protocol of its own over control transfers,
  which NeOS rules out (NeOS ADR 0060).
- **CDC Ethernet's, with RNDIS's descriptors** _(chosen)_: the card describes both,
  byte for byte, so that NeOS's choice between them is tested, and choosing RNDIS
  halts Nebu.

### How the hub gets its device

- **A list of devices, one a port.** More than any scenario uses.
- **One device on port 1** _(chosen)_, a keyboard or the network card, as each
  scenario plugs in one. Asking for both halts Nebu's start with a message.

## Decision

1. **The mailbox answers tag 0x0001_0003** with 52:54:00:12:34:56, its six bytes in
   order from the value's first byte, and a length of 6.
2. **The DWC2 runs bulk transfers.** A bulk packet the device NAKs leaves the channel
   enabled, sets HCINT's NAK bit, and keeps what is left of the transfer; Nebu tries
   it again after each transfer on another channel. Disabling the channel halts it,
   and it is not tried again. A NAK in a control transfer still halts Nebu, since
   Nebu's devices never send one.
3. **A new `usb_network` module** models QEMU's `usb-net` in its CDC Ethernet
   configuration, with QEMU's device and configuration descriptors and strings, and
   its MAC address string, 405400123457. It takes Set Interface for the data
   interface's settings 0 and 1, and Set Ethernet Packet Filter. Bulk transfers before
   the data interface's setting 1 is chosen, choosing RNDIS, and reading the status
   endpoint halt Nebu.
4. **Its network** answers an ARP request for 10.0.2.2 or 10.0.2.3 with slirp's
   64-byte reply, ignores other ARP, and halts Nebu on any other frame.
5. **`--network`** puts QEMU's hub on the USB port with the card on its port 1, as
   `--keyboard` does with the keyboard. The two cannot be given together.

## Consequences

- Every NeOS kernel from this stage on runs in Nebu, and the `network` scenario runs
  in both emulators, with the same output.
- NeOS's bulk transfers, its choice among configurations, its alternate settings, and
  zero-length packets both ways are checked against a second model of the hardware.
- Nebu's network grows with NeOS's: DHCP, then TCP, will each need a reply here, or
  the scenarios that use them will halt Nebu, loudly, until they have one.
- A scenario that wants a keyboard and a network card at once will need the hub to
  take more than one device.
