# 0036. The watchdog, and restarting the machine

- **Status:** Proposed
- **Date:** 2026-11-04

## Context

NeOS ADR 0059 updates the system with two slots, a boot count and the watchdog. The
chainloader arms the watchdog before it starts a slot on trial, the update service
disarms it once the new system is healthy, and arming it with a short count is how
NeOS restarts the Pi. If the count runs out, the whole chip resets, the firmware loads
`kernel8.img` and `neos.img` again, and the chainloader reads the boot record it
wrote on the card before the reset.

The watchdog lives in the power manager at 0x3F10_0000. Three registers matter:

- `RSTC` (0x1C): bits 5:4 configure what happens when the count runs out. Writing
  0b10 ("full reset") arms the watchdog; writing 0x102, as Linux does, disarms it.
- `RSTS` (0x20): what caused the last reset, and a partition number the firmware
  reads. NeOS does not write it.
- `WDOG` (0x24): the count, in ticks of 1/65,536 of a second, in the low 20 bits.
  Writing it again while armed starts the count again.

Every write must carry the password 0x5A in its top byte, or the hardware ignores it.

QEMU 8.2 models the registers but not the count: writing "full reset" to `RSTC`
resets the machine at once. QEMU after version 10.2 counts down from `WDOG` with a
timer and resets when it reaches zero. Nebu halts on any device it does not model, so
today NeOS's update service, and every restart, would stop it.

## Options

### How the count runs

- **Reset at once when armed**, as QEMU 8.2 does. Simple, but a trial could never
  succeed, and NeOS's update scenario would have nothing to test against.
- **Count in emulated time** _(chosen)_, as the hardware and newer QEMU do. Nebu's
  counter is one count per instruction at 62.5 MHz (ADR 0016), so a tick is
  62,500,000 / 65,536 counts, rounded to 954. When every core waits, Nebu already
  skips to the next interrupt that is due (ADR 0017); the watchdog's deadline is
  added to what it skips to, so a kernel that hangs in `wfi` reaches the reset at
  once rather than after sixteen seconds of waiting.

### What a reset does

- **Stop, and let the test start Nebu again.** Each boot would be a run of its own.
  But a scenario that restarts by itself would need `xtask` to notice the reset and
  start Nebu again with the same card, and the transcript of one test would be split
  across runs, which is not what happens on the Pi.
- **Start the machine again in the same run** _(chosen)_, as QEMU does. RAM is
  cleared, the kernel image and every `--load` file are loaded again where they were,
  as the firmware would load them, every core and device starts as it did at power-on,
  and the counter starts again from zero. The SD card keeps what was written to it,
  and serial output carries on in the same stream, so one transcript holds every boot.
- **Keep RAM as it was.** The real chip's memory may well survive a watchdog reset,
  but nothing promises it, and NeOS must not rely on it. Clearing it makes a kernel
  that does rely on it fail in Nebu first.

### Unknown writes

- **Ignore a write without the password**, as the hardware does. Faithful, but a
  driver bug would pass silently.
- **Halt on it** _(chosen)_, as Nebu does for anything it does not model, saying the
  password was missing. Any other configuration of `RSTC`'s bits 5:4 than "clear" and
  "full reset", a write to `RSTS`, and any register but these three halt too.

## Decision

1. **A new `power` module** models the power manager at 0x3F10_0000, with a 4 KiB
   window, holding `RSTC`, `RSTS` and `WDOG`. Reads return what was last written (with
   the password removed); `RSTS` reads as 0x1000 after a power-on and 0x1020 after a
   watchdog reset, as QEMU's power-on value and the hardware's "had a watchdog reset"
   bit.
2. **Arming**: a write to `RSTC` whose bits 5:4 are 0b10, or a write to `WDOG` while
   armed, sets the deadline to now plus the count times 954. A write of bits 5:4 = 0
   disarms it. A count of 0 never resets.
3. **The reset**: when the counter reaches the deadline, Nebu prints nothing, clears
   RAM, loads the image and the `--load` files again, makes every core and device new
   except the SD card, and goes on from counter 0. The machine's main loop, when every
   core waits, skips to the watchdog's deadline as it does to a timer interrupt.
4. **Tests** check that a write without the password halts, that arming and writing
   `WDOG` again moves the deadline, that disarming clears it, and that `RSTS` says
   whether the watchdog reset the machine. NeOS's update scenarios check the reset
   itself: that the image is loaded again and the card keeps what was written to it.

## Consequences

- NeOS's update service and its restarts run under Nebu, and the update scenario can
  show a trial that succeeds, which QEMU 8.2 cannot.
- A run can now hold several boots. A kernel that restarts for ever runs until
  `xtask`'s timeout, as one that loops does.
- Nebu still models nothing else in the power manager: power domains, the clock
  manager beside it at 0x3F10_1000, and the partition number in `RSTS` that the
  firmware would read.
