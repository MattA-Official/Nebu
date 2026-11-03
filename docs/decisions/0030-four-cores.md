# 0030. Four cores

- **Status:** Proposed
- **Date:** 2026-11-03
- **Amends:** [0006](0006-machine-model.md) (one core), [0016](0016-the-timer-and-interrupts.md)
  (one count for each instruction) and [0017](0017-device-interrupts-and-the-system-timer.md)
  (how `wfi` skips ahead)

## Context

NeOS now starts the BCM2837's other three cores
([NeOS ADR 0051](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0051-running-on-all-four-cores.md)
plans the area, and
[NeOS ADR 0052](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0052-starting-the-other-cores.md)
is this stage). The kernel:

- writes the physical address of `_start_secondary` into the firmware's spin table,
  at 0xE0, 0xE8 and 0xF0 for cores 1 to 3, cleans those words to RAM with `dc civac`,
  and sends an event with `sev`;
- on each new core, at EL2, copies `MPIDR_EL1` and `MIDR_EL1` into `VMPIDR_EL2` and
  `VPIDR_EL2`, which are what EL1 reads them as;
- on each new core, reads `MPIDR_EL1` for the core's number, drops from EL2 to EL1,
  turns on the MMU, and stores to a flag with `stlrb` (store-release) to say it is
  running. The boot core waits for the flags with `ldarb` (load-acquire) and reads
  the counter, for up to a second;
- stores the kernel's page table addresses with `stlr` and loads them with `ldar`;
- forgets mappings on every core with `tlbi vmalle1is` and `tlbi aside1is`, and
  empties every core's instruction cache with `ic ialluis`;
- parks the new cores in `wfe`.

Nebu has one core, which `MPIDR_EL1` says is core 0. It cannot decode `sev`,
`ldar`, `stlr` or the broadcast `tlbi`s. Its counter counts the one core's
instructions, and `wfi` moves the counter on to the next interrupt due (ADRs 0016
and 0017). `wfe` with no event stops the run, since nothing could ever send one.

QEMU starts every core of `raspi3b`, and runs the three it does not boot in a stub
it writes at 0x300: each core waits with `wfe`, reads its word in the spin table at
0xD8 plus 8 times its number, and jumps there with x0 to x3 cleared once it is not
zero. The Pi's firmware does the same in its `armstub8`. Both start the cores at
EL2.

## Options

### How many cores, and how they take turns

- **Four cores, one instruction from each in turn** _(chosen)_. Each count of the
  counter runs one instruction on every core that is running, in the order 0, 1, 2,
  3. Every run of the same image with the same input does the same thing, so a test
  that fails fails every time, and a trace reads in a fixed order. A core waiting in
  `wfe`, `wfi` or the stub costs nothing, so a kernel that parks three cores runs as
  fast as before, and a core counts time at the same rate whether or not the others
  run.
- **Four cores on four host threads**, as QEMU can. Faster with four busy cores, and
  it would find races. But Nebu would need locks around the bus and memory, its runs
  would not repeat, and it would lose what makes it worth having beside QEMU: QEMU
  already runs cores in parallel. Nebu has no `unsafe`, so sharing memory between
  host threads would cost a lock on every access.
- **More than one instruction from each core before the next**, a quantum. Faster,
  since a core's state stays hot, and other quanta would try other orders. It can be
  added later as an option, with one instruction as the default.
- **One core**, as now, and ignore the others. The kernel would wait a second for
  cores that never start, and every multi-core stage after this one would have
  nothing to test against but QEMU.

### The firmware's stub

- **Run QEMU's stub code**, written into memory at 0x300 as QEMU does. Nebu would run
  exactly the instructions QEMU does, but it would need `ldr` with a shifted register
  index, and memory at 0x300 would hold code that NeOS never loaded.
- **Model the stub in Nebu** _(chosen)_. A core in the stub waits for an event; on
  one, it reads its word in the spin table, and if it is not zero, starts there at EL2
  with every exception masked, x0 to x3 zero, x4 the address, x5 the spin table's
  address and x6 its number, as QEMU's stub leaves them. Nebu already models the
  firmware this way for the mailbox (ADR 0011).

### What `wfi` and `wfe` do

- **A core that waits sleeps**, and is woken: from `wfi` when an interrupt is pending
  for it, from `wfe` or the stub by an event. While any core runs, the counter
  counts as usual. Only once every core waits does Nebu skip the counter to the next
  interrupt due, or wait for input, or, with nothing due, say every core is waiting
  and sleep for ever, as before. With one core running, this is exactly what Nebu
  did before.
- **`sev` sets the event register of every core, its own too**, as the architecture
  says. `wfe` with the event register set clears it and carries on.

### Interrupts and timers on cores 1 to 3

- **Only core 0 has them, for now** _(chosen)_. The kernel does not yet use the other
  cores' timers or IRQs, and the local block's model (ADR 0016) has only core 0's
  registers. A core other than 0 that enables its timer halts Nebu, as anything else
  Nebu does not emulate does. NeOS ADR 0053 will need each core's timer, and that
  stage's Nebu ADR adds them.

### The new instructions

- **`sev`**, and **`tlbi vmalle1is`, `tlbi aside1is` and `ic ialluis`**, which do
  nothing more than their single-core forms, since Nebu has no TLB or caches (ADR
  0009).
- **`MIDR_EL1`, `VPIDR_EL2` and `VMPIDR_EL2`.** `MIDR_EL1` reads as QEMU's
  Cortex-A53 r0p4 (0x410FD034). The two EL2 registers always hold the real IDs, so
  EL2 may write them only with those; any other value halts Nebu.
- **`ldar`, `ldarh`, `ldarb`, `stlr`, `stlrh` and `stlrb`**, as ordinary loads and
  stores. Nebu runs one instruction at a time and has no caches, so every access is
  already ordered. They must be aligned, as on hardware; a misaligned one halts Nebu.

## Decision

1. Nebu has four cores. Each count of the counter runs one instruction on each core
   that is running, in the order 0 to 3.
2. Cores 1 to 3 start in a model of the firmware's stub, and leave it at EL2 for the
   address in their word of the spin table at 0xD8, once an event finds it set.
3. `MPIDR_EL1` gives each core's number in its low byte.
4. `wfi` and `wfe` put a core to sleep until an interrupt is pending for it, or an
   event. `sev` sends an event to every core. Nebu skips the counter ahead only when
   every core sleeps.
5. Only core 0 takes interrupts and has a timer. Another core that enables its timer
   halts Nebu.
6. Nebu decodes `sev`, `tlbi vmalle1is`, `tlbi aside1is`, `ic ialluis`, and the
   load-acquire and store-release instructions of every size, and models `MIDR_EL1`,
   `VPIDR_EL2` and `VMPIDR_EL2`.
7. A fault names the core it happened on.

## Consequences

- NeOS's stage 1 runs the same in Nebu as in QEMU: four cores start and wait.
- Runs repeat exactly, so a multi-core failure in Nebu can be traced and fixed. Races
  that only some orders show need QEMU or the Pi, or a quantum option later.
- With one core running and three waiting, Nebu is as fast as before. With four
  running, it is a quarter of the speed per core.
- The next stages add each core's timer and interrupts, the local block's mailboxes,
  and exclusive loads and stores (`ldaxr` and `stlxr`), each in its own ADR.
