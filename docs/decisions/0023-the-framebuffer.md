# 0023. The framebuffer

- **Status:** Accepted
- **Date:** 2026-10-22
- **Amends:** [0011](0011-the-firmware-mailbox.md), decision 3 (only GET_ARM_MEMORY
  is answered)

## Context

NeOS now draws text on the screen
([NeOS ADR 0036](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0036-the-screen.md)).
At boot its kernel asks the firmware for a framebuffer through the mailbox's
property interface: "get physical display size", then one message with "set
physical size", "set virtual size", "set depth" 32, "set pixel order" RGB, "set
virtual offset" 0, "allocate buffer" and "get pitch". A user-space display driver
then writes pixels into the buffer, which the GPU shows.

Nebu's mailbox answers only "get ARM memory" (ADR 0011), and stops on any other tag,
so NeOS could not boot in it. QEMU's `raspi3b` answers the framebuffer tags with a
640×480 screen, and its buffer at 0x3C10_0000, and its monitor's `screendump` saves
the screen as a PPM image. `cargo xtask test` reads such images back as text to check
what NeOS drew.

## Options

### Answering the tags

- **Refuse them**, so NeOS's kernel finds no screen. Its display driver would draw
  nothing, and nothing on the screen could be tested in Nebu.
- **Answer every value NeOS might ask for**: other depths, a virtual size larger than
  the physical one, offsets. More code for requests NeOS never makes.
- **Answer what NeOS asks for, as QEMU does, and stop on anything else**
  _(chosen)_, as ADR 0001 says. 640×480 by default, as QEMU; any size NeOS sets up to
  4096×4096; 32 bits per pixel only; virtual size equal to the physical size; offset
  0. The buffer is at QEMU's address, 0x3C10_0000, in the GPU's share of RAM, which
  Nebu's RAM already covers.

### Showing the screen

- **A window.** The standard library has no way to open one, and ADR 0002 rules out
  crates.
- **Save the screen to a file when the machine stops** _(chosen)_: `--screen
  <file.ppm>`. PPM is the simplest image format there is, a short text header then
  the pixels, three bytes each, and is what QEMU's `screendump` writes, so xtask reads
  both the same way.
- **Save it on a signal, or every so often.** Lets a person watch, but needs a way to
  ask that the standard library does not give. Tests only need the end.

## Decision

1. The mailbox keeps a framebuffer's settings: its width and height (640×480 to
   start), depth, pixel order, and whether it has been allocated. It answers
   "get physical display size", "set physical size" (1 to 4096 each way), "set
   virtual size" (only the physical size), "set depth" (only 32), "set pixel order"
   (0 or 1), "set virtual offset" (only 0), "allocate buffer" (only at 32 bits per
   pixel; the bus address 0x3C10_0000 and the size, the pitch times the height) and
   "get pitch" (the width times 4). Anything else stops Nebu, naming what it cannot
   do.
   Setting the physical size again after "allocate buffer" undoes the allocation, so
   the buffer must be allocated again for the new size.
2. `--screen <file.ppm>` saves the framebuffer as a binary PPM (`P6`) image when the
   machine stops: on a semihosting exit, when Nebu stops at something it cannot
   emulate, or when the core goes to sleep with nothing to wake it, so that the
   screen can help work out what happened. If no buffer is allocated, Nebu says so
   on standard error and saves nothing.
3. Pixels are read from RAM as little-endian 32-bit words, as QEMU's `bcm2835_fb`
   reads them: with pixel order RGB, red is the word's bits 7:0 and blue bits 23:16;
   with BGR, the other way round. NeOS draws only greys so far, in which the two are
   the same, so this has not been checked against a picture from QEMU or the Pi.

## Consequences

- NeOS boots in Nebu with a screen, and `cargo xtask test` checks what its shell
  test leaves on the screen in both emulators.
- The framebuffer is plain RAM, so drawing costs Nebu nothing extra; saving it is
  done once, at the end.
- A NeOS that asks for another depth, a larger virtual size, or a new tag stops Nebu,
  which is when Nebu should learn it.
