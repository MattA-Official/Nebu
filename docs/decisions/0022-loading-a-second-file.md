# 0022. Loading a second file into memory

- **Status:** Accepted
- **Date:** 2026-10-22

## Context

NeOS now boots on its own when no PC answers the chainloader
([NeOS ADR 0031](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0031-booting-without-a-pc.md)).
On the Pi, `config.txt`'s `initramfs neos.img 0x3000000` has the firmware load the
card's kernel at 48 MiB before the chainloader starts. QEMU does the same with its
generic loader, `-device loader,file=neos.img,addr=0x3000000`. Nebu loads only
`kernel8.img`, at 0x80000, so it cannot run the new `standalone` scenarios.

## Options

- **Read `config.txt` from the SD card image**, as the firmware does. Truest to the
  Pi, but Nebu would need a FAT32 reader and a parser for `config.txt`, only to find
  one line, and QEMU, the reference, does not do it either.
- **A fixed second file**, such as `neos.img` next to `kernel8.img`. No option to
  add, but a hidden rule the tests would have to know about, and fixed to one
  address.
- **An option naming a file and an address** _(chosen)_: `--load <file>@<address>`,
  which may be given more than once. It does what QEMU's generic loader does, so
  xtask passes the same file and address to both.

## Decision

1. `--load <file>@<address>` copies the file into RAM at the address before the first
   instruction runs. The address is hexadecimal, with `0x`. The option may be repeated.
2. Files are loaded after `kernel8.img`, in the order given, so a later one wins
   where they overlap. A file that does not fit in RAM stops Nebu before it starts,
   with exit code 2, as an unreadable `kernel8.img` does.

## Consequences

- Nebu runs NeOS's `standalone` and `standalone-damaged` scenarios, with the same
  output as QEMU.
- No instructions or devices change.
