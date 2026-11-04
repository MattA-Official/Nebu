# 0032. The hardware random number generator

- **Status:** Accepted
- **Date:** 2026-11-03

## Context

NeOS's random service reads the BCM2835's hardware random number generator at
0x3F10_4000 and mixes its words, with timing jitter, into a ChaCha20 generator
(NeOS ADR 0047). Nebu halts on any peripheral it does not model, so without this
the random service cannot run under Nebu, and `cargo xtask test`'s `random`
scenario cannot run there.

The generator is not documented by Broadcom, and what is inside it is not public.
Nebu therefore has nothing to copy faithfully; what it must get right is how the
registers behave, since that is all NeOS can see:

- `RNG_CTRL` (0x0): bit 0 starts it.
- `RNG_STATUS` (0x4): bits 31 to 24 count the words waiting. The low 20 bits are a
  warm-up count, which a driver writes before starting it.
- `RNG_DATA` (0x8): reading takes the next word.
- `RNG_INT_MASK` (0x10): bit 0 masks its interrupt.

QEMU models the first three, and answers every read of `RNG_DATA` from the host's
own random source, with the status register always saying one word is waiting. It
does not model the interrupt mask at all, which NeOS's service writes; the write
lands outside QEMU's register window and is ignored.

Two things make this different from Nebu's other devices. There is no right answer
to check against: a test cannot assert what word the generator gives. And NeOS
checks what it reads with SP 800-90B's health tests, so words that repeat, or that
lean on a few values, make the service exit.

## Options

### Where the words come from

- **A fixed sequence**, the same on every run, as Nebu's timing is. It makes a run
  repeatable, which is how the rest of Nebu works, and `cargo xtask test` could then
  check exact output. But NeOS's own test checks that two reads differ and that two
  clients differ, which a fixed sequence would still pass while hiding the thing most
  worth catching: a service that hands the same bytes to everyone. It would also make
  the emulator a poor place to try anything that leans on randomness.
- **The host's randomness**, as QEMU does, through `/dev/urandom` or
  `getrandom`. Closest to the hardware, but it needs a file or a system call for
  something Nebu can work out itself, and makes a run impossible to repeat.
- **A generator of Nebu's own, seeded from the host's clock** _(chosen)_. SplitMix64
  is a dozen lines, needs nothing outside `std`, spreads its output evenly enough to
  pass NeOS's health tests and its evenness check, and differs on every run because
  the seed does. A run can be repeated by fixing the seed, if that is ever wanted. It
  is not meant to stand up to anyone trying to guess it, and it does not have to:
  NeOS mixes what it reads into its own generator, and treats the hardware as one
  source among others (NeOS ADR 0047).

### What the status register says

- **Count the words as the hardware does**, filling a queue over time. More faithful,
  but it would need a notion of how fast the generator runs, which is not documented,
  and would only make a driver wait.
- **Always one word waiting** _(chosen)_, as QEMU says. A driver that polls reads one
  word each time round, and one that waits never waits long. The warm-up count written
  to the low bits is kept and read back, so a driver can see its own write.

### The interrupt mask

- **Leave it out**, as QEMU does: NeOS's write would then halt Nebu, since it halts
  on registers it does not model. That would make Nebu stricter than the hardware
  about a register the hardware has.
- **Keep it, and read it back** _(chosen)_. It costs one field. Nebu raises no
  interrupt from the generator either way, since NeOS masks it and polls, so nothing
  depends on what is in it.

## Decision

1. **A new `rng` module** models the generator at 0x3F10_4000, with a 4 KiB window,
   as the other devices have.
2. **`RNG_CTRL` and `RNG_INT_MASK`** read back what was written. `RNG_STATUS` reads
   back the warm-up count that was written, with bit 24 set, so one word always
   waits. Any other register, or any access that is not 32 bits, halts Nebu as usual.
3. **`RNG_DATA`** gives the next word of a SplitMix64 generator seeded from the
   host's clock, whether or not the generator was started: the real one's queue may
   hold words from before it was stopped, and a driver that forgets to start it is
   not what Nebu is there to catch.
4. **Tests** check that the registers read back, that an unimplemented register is
   refused, that each word differs from the one before, that the bytes of 4,096 words
   are spread evenly over all 256 values, and that two generators made a moment apart
   give different words.

## Consequences

- NeOS's random service, and the `random` scenario, run under Nebu as well as QEMU.
- Nebu's output for that scenario cannot be compared with QEMU's word for word, which
  is why NeOS's test prints no random value.
- A run that uses random numbers cannot be repeated exactly unless the seed is fixed,
  which nothing does yet.
- Anything that wants to try NeOS's encryption or signing under Nebu has numbers to
  work with, without the host's random source.
