# 0028. Writing to the SD card

- **Status:** Accepted
- **Date:** 2026-11-03
- **Amends:** [0019](0019-the-sd-card.md), which said the card is only ever read

## Context

NeOS's file system can now write
([NeOS ADR 0032](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0032-writing-to-the-file-system.md)).
Its SD card driver writes one block with CMD24, and several with CMD25, ended by
CMD12, using the controller's block counter. Nebu halts on any write, and opens the
card image for reading only. NeOS's tests boot twice on one card, and check the card
image after Nebu stops, so what NeOS writes must reach the image file.

How QEMU 8.2's controller (`hw/sd/sdhci.c`) takes a write, which Nebu matches: the
command sets "write transfer active", "data inhibit" and "buffer write enable" in
STATUS and raises the "buffer write ready" interrupt (bit 4). After each 512 bytes
written to DATA, it counts the block down when the block counter is enabled; when
the count reaches 0, or for a single block, the transfer finishes with "transfer
complete" (DATA_DONE), and otherwise "buffer write ready" is raised again. A command
with a busy response finishes with DATA_DONE at once, as Nebu's CMD7 already does.

## Options

For when the image file is written:

- **Keep the writes in memory and write them back when Nebu stops**, as NeOS ADR
  0032 first described. Fewer file writes, but a crash of Nebu, or of the host, loses
  them, and Nebu would need a map of changed blocks.
- **Write each block to the file as it arrives** _(chosen)_. Simplest, and the file
  matches the card at every moment, as a real card does once a busy card lets go.
  The host's own cache makes it as fast as NeOS's tests need.

For what the controller models:

- **Single-block writes only.** NeOS writes several blocks in a row with CMD25, as
  NeOS ADR 0032 decides, so this would halt.
- **CMD24, and CMD25 with the block counter, ended by CMD12** _(chosen)_. Nebu still
  halts on multi-block reads, Auto CMD12 and Auto CMD23, which NeOS does not use.

## Decision

1. `--sd` opens the image for reading and writing, or for reading only if it may not
   be written, in which case a write halts Nebu. A block is written to the file as
   soon as its last word reaches DATA.
2. CMD24 (one block) and CMD25 (several, until CMD12) are accepted in the transfer
   state, for blocks within the card. A write takes the card to the receive-data
   state, which CMD12 (and, for CMD24, the end of its one block) leaves.
3. During a write, STATUS shows DAT_INHIBIT and WRITE_AVAILABLE (bit 10), and
   INTERRUPT bit 4 (WRITE_RDY) is raised for each block. The last block raises
   DATA_DONE; the card is never busy afterwards.
4. CMD13, the card's status, is answered in the transfer state. NeOS asks for it
   after each write until the card is ready for data; Nebu's card always is.
5. CMDTM's multi-block and block-count-enable bits are accepted, only together, and
   only for a write. A command sent before a write's blocks are all written halts
   Nebu, as does writing DATA with no write under way. Resetting the data line
   abandons a write; a card part way through CMD24's block goes back to the transfer
   state, and one in the middle of CMD25 still waits for CMD12. BLKSIZECNT's count
   goes down by one for each of CMD25's blocks, as QEMU's does. A write that would
   run past the end of the card halts Nebu before any of it is written.

## Consequences

- NeOS's `writing` and `power-cut` scenarios run in Nebu, and the image file holds
  what NeOS wrote once Nebu stops.
- An image used with `--sd` is changed; NeOS's tests make a fresh image for each run.
- The card's cache, erase commands and write protection stay unmodelled; NeOS uses
  none of them.
