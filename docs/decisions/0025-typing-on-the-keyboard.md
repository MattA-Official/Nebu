# 0025. Typing on the keyboard

- **Status:** Proposed
- **Date:** 2026-10-23
- **Amends:** [0024](0024-the-usb-controller-and-keyboard.md), decision 9 (the
  keyboard's report endpoint)

## Context

NeOS's keyboard stage
([NeOS ADR 0039](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0039-the-usb-keyboard.md))
adds a keyboard driver that reads reports from the keyboard's endpoint 0x81 and
turns them into key presses and releases. Its `keyboard` scenario types on the
keyboard and checks what the shell prints.

**In QEMU**, xtask types through QMP's `input-send-event`, one press or release at a
time, with QEMU's names for keys (its `QKeyCode`s: `shift`, `h`, `caps_lock`, `esc`).
QEMU 8.2's keyboard (`hw/input/hid.c`) queues each as a key code, and each time the
host polls its endpoint with something queued it takes one, updates the keys held,
and sends a report: the modifier keys as bits in byte 0, a reserved byte, and the
first six other keys held in the order they were pressed, or 1 in all six if more are
held. With nothing queued, and Set Idle at 0, it answers NAK. A USB reset empties the
queue.

Nebu has no monitor to type through. ADR 0024's keyboard always answers NAK.

## Options

- **Read keys from a second input**, such as a file or a socket, while Nebu runs.
  Closest to QEMU, but Nebu reads only standard input, which is the serial port, and
  a second stream would need a thread and a protocol of its own.
- **A list of keys on the command line, typed one a poll** _(chosen)_. xtask knows
  every key a scenario types before it starts, so it can pass the same list it sends
  QEMU. Each poll that finds a key waiting takes one, as QEMU's does, so the reports
  NeOS sees are the same, only sooner: NeOS's driver keeps what comes until it is
  read.
- **Keys at set times**, counted in the core's counter. Matches when a person types,
  but NeOS keeps nothing that depends on when keys come, and choosing the times would
  tie the scenario to how long Nebu takes to boot.

## Decision

1. **`--keys <list>`** plugs in the keyboard, as `--keyboard` does, and lists what is
   typed on it: names separated by commas, each after `+` for a press or `-` for a
   release, such as `+shift,+h,-h,-shift`. The names are QEMU's: `a` to `z`, `0` to
   `9`, `ret`, `esc`, `backspace`, `tab`, `spc`, the punctuation keys, `caps_lock`,
   `num_lock`, `home`, `end`, `delete`, the four arrows, and `ctrl`, `shift` and `alt`
   with their `_r` forms. Any other name is an error.
2. **The keyboard's report endpoint** answers each poll, while a press or release is
   waiting, by taking one and sending the report that follows, built as QEMU builds
   it. With none waiting, it answers NAK, as before.
3. **A reset** forgets the keys held, the protocol and the idle rate, as in QEMU, but
   not the presses and releases still to come: they stand for a person who carries on
   typing.

## Consequences

- The keyboard scenario passes the same list to QEMU, through QMP, and to Nebu, and
  NeOS prints the same.
- The keys arrive as fast as NeOS polls, so Nebu does not test what NeOS does when
  keys come while no one is reading them, beyond its buffer.
