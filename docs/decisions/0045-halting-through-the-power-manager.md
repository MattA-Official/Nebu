# 0045. Halting through the power manager

- **Status:** Proposed
- **Date:** 2026-11-09

## Context

NeOS ADR 0083 adds `shutdown`. Once every service has stopped, the root task halts
the Pi as Linux's `bcm2835_wdt` driver powers it off: it reads the power manager's
reset status register, `RSTS` (0x3F10_0020), sets the partition bits to 63, writes it
back with the password, and arms the watchdog with the shortest count. When the count
runs out the chip resets, the firmware reads partition 63, and stays halted until the
power is cycled. A restart is the same without the write to `RSTS`.

`RSTS` holds the partition the firmware boots from in its even bits 0 to 10 (0x555
for all six). Nebu ADR 0036 made `RSTS` read-only and halts on a write to it, so
`shutdown` would stop Nebu with an error, and the run's exit code would say it
failed.

QEMU 8.2 stores what is written to `RSTS`, and when "full reset" is written to `RSTC`
with `RSTS & 0xFFF` equal to 0x555 it shuts the machine down, ending the run with
exit code 0, instead of resetting it.

## Options

### What a halt does

- **Reset as usual, and let NeOS start again.** Nebu has no firmware to read the
  partition, but a run would never end, and a `shutdown` scenario would run until
  `xtask`'s timeout.
- **End the run with exit code 0 when the watchdog's count runs out with partition
  63 in `RSTS`** _(chosen)_. This is what the firmware does, seen from outside: the
  machine stops for good. QEMU stops when `RSTC` is written instead, a few ticks
  earlier, but NeOS does nothing between the two, so the output is the same.
- **End the run as soon as `RSTS` names partition 63.** Earlier than both the chip
  and QEMU, and a kernel that wrote it and then went on would be cut short.

### Which bits name partition 63

- **`RSTS & 0xFFF == 0x555`**, as QEMU checks. After a watchdog reset Nebu's `RSTS`
  also has bit 5 set (ADR 0036), which QEMU never sets, so Linux's way of writing it
  would not halt.
- **The partition bits only, `RSTS & 0x555 == 0x555`** _(chosen)_, as the firmware
  reads them.

## Decision

1. **`RSTS` is writable**, with the password, as `RSTC` and `WDOG` are. It reads back
   as written, without the password.
2. **When the watchdog's count runs out** and `RSTS`'s partition bits are all set,
   Nebu saves the screen if it was asked to, prints `nebu: the machine halted` on
   standard error, and exits with code 0. Otherwise the reset is as ADR 0036 says.
3. **Tests** check that a write without the password is still refused, that partition
   63 halts and another does not, and that writing `RSTS` does not arm the watchdog.
   NeOS's `shutdown` scenario checks the halt itself.

## Consequences

- NeOS's `shutdown` ends a run in Nebu as it does in QEMU, and `restart` still resets.
- Nebu still models nothing else of `RSTS`: the other bits are kept as written, and
  nothing reads them.
