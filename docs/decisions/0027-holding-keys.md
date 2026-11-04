# 0027. Holding keys

- **Status:** Accepted
- **Date:** 2026-11-03
- **Amends:** [0025](0025-typing-on-the-keyboard.md), decisions 1 and 2

## Context

NeOS's timers
([NeOS ADR 0043](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0043-timers-for-programs.md))
make a held key repeat: the console types it again 500 ms after it is pressed, then 25
times a second, until it is released. Its `repeat` scenario holds a key down and checks
that it was typed more than once.

ADR 0025 types `--keys` one press or release a poll, as fast as NeOS polls, and chose
that over keys at set times because "NeOS keeps nothing that depends on when keys
come". That is no longer so: a key released at the next poll after its press never
repeats.

In QEMU, xtask holds a key by waiting between the press and the release that it sends
over QMP. While nothing changes, QEMU's keyboard answers NAK, since Set Idle is 0.

NeOS's USB driver polls the keyboard's endpoint every 10 frames, as its descriptor
asks, and Nebu's frames are a millisecond of the core's counter (ADR 0024), so a poll
in Nebu stands for 10 ms of the machine's time, as in QEMU.

## Options

- **A pause in milliseconds of the core's counter.** Says what a person means, but
  the keyboard is handed packets, not the time, so it would need the controller to
  pass the frame number to every device.
- **A pause in polls** _(chosen)_. `~N` answers the next `N` polls NAK, with every key
  held as it is. The keyboard needs only a count. xtask knows the poll interval, so
  it can turn the same item into a wait for QEMU.
- **Repeat in Nebu's keyboard**, sending the held key again. Keyboards do not do
  that; repeat is the host's, and testing it is the point.

## Decision

1. **`--keys` takes `~N`** beside presses and releases: the keyboard answers the next
   `N` polls of its report endpoint NAK, with every key held as it is, then carries on
   with the list. `N` is a whole number; anything else after `~` is an error.

## Consequences

- NeOS's `repeat` scenario runs in Nebu as in QEMU, with the same list.
- A pause counts polls, so it lasts as long as NeOS's polling says. If NeOS polled
  more often, the same pause would be shorter, and the scenario's count would need to
  change with it.
