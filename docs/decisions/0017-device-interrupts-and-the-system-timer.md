# 0017. Device interrupts and the system timer

- **Status:** Proposed
- **Date:** 2026-10-21
- **Amends:** [0006](0006-machine-model.md) (its list of devices) and
  [0016](0016-the-timer-and-interrupts.md) (decision 3, what the local block and
  the BCM2835 controller accept)

## Context

NeOS stage 6b lets user-space drivers reach their devices' registers and receive
their interrupts
([NeOS ADR 0022](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0022-device-memory-and-interrupts.md),
proposed in NeOS #17). The kernel now:

- writes 0 to the BCM2835 controller's FIQ control (0x3F00_B20C), so no interrupt
  is a FIQ, and 0 to the local block's GPU routing (0x4000_000C), so the BCM2835
  line reaches core 0 as an IRQ;
- enables and disables device interrupts through the controller's enable
  registers (0x3F00_B210 and 0x3F00_B214) and disable registers (0x3F00_B21C and
  0x3F00_B220), for whichever interrupts drivers have bound;
- on an IRQ with the GPU bit (bit 8) of core 0's IRQ source set, reads the
  controller's two pending registers (0x3F00_B204 and 0x3F00_B208) and handles the
  interrupts it enabled;
- runs `wfi` with IRQs masked when no thread can run but an interrupt is enabled,
  after turning the virtual timer off.

User programs also map device memory now, as Device-nGnRnE at EL0. Nebu's page
table walk already treats MAIR attribute 0x00 as Device memory, and lets EL0 reach
any page its entry allows, so that needs nothing new.

NeOS's `interrupts` test maps the BCM2835 system timer at 0x3F00_3000, sets
comparator 1 5 ms ahead of its counter, and waits for comparator 1's interrupt
(interrupt 1), first with no thread to run and then while a thread counts at EL0.
Nebu halts on all of this today.

## Options

### How the system timer counts

The system timer is a 64-bit counter at 1 MHz, with four 32-bit comparators. A
comparator raises its interrupt, and sets its bit in the status register, when the
counter's low word becomes equal to it. The bit stays set until written with 1.

- **From the host's clock.** It would follow real time, but runs could not be
  repeated, which is why the generic timer does not (ADR 0016).
- **One count for each instruction.** It is deterministic, but the system timer
  would run 62.5 times faster than the generic timer, though on the Pi and in QEMU
  both follow the same time. A program that measured one against the other would
  see something no machine does.
- **From the core's counter, 62.5 of its counts to each of the system timer's**
  _(chosen)_. Both timers then agree, as they do on the Pi, and a run is still
  repeatable. A 5 ms delay is 312,500 instructions.

### Which device interrupts to model

- **All 64 of the BCM2835's interrupts**, accepting any enable. Nebu raises none
  but the system timer's, so the rest would be enabled and never fire, and a kernel
  that waited on one would hang without saying why.
- **The system timer's four comparators.** Comparators 0 and 2 are the GPU
  firmware's on the Pi, and NeOS refuses handlers for them, so enabling them could
  only be a bug.
- **Comparators 1 and 3 only** _(chosen)_. Enabling any other interrupt halts
  Nebu, so a kernel that starts using another device finds out at once (ADR 0001).

### What the pending registers show

- **Interrupts that are both raised and enabled.** It is simpler for a kernel, but
  QEMU's `raspi3b` shows raised interrupts whether or not they are enabled, and the
  BCM2835 manual does not settle what the Pi does. A kernel tested only against
  this would never learn to ignore bits it did not enable.
- **Every raised interrupt, enabled or not** _(chosen)_, as QEMU shows them. NeOS
  masks what it reads with what it enabled (NeOS ADR 0022, decision 10), and this
  checks that it does. The BCM2835 line itself is raised only while a raised
  interrupt is enabled.

### How `wfi` waits

`wfi` pauses the core until an interrupt is pending, even one PSTATE.I masks, and
the kernel runs it only when nothing else can run.

- **Run nothing, one instruction's time at a time**, until an interrupt is
  pending. It is easy to see that it is right, but a long wait costs as much as
  running that many instructions, and a wait with nothing due would spin for ever.
- **Skip the counter straight to the next interrupt due** _(chosen)_. Nebu works
  out when the virtual timer next fires, if it is enabled, unmasked and routed, and
  when the next enabled comparator matches, and moves the core's counter to the
  earlier. The wait is then charged to the counter, as real time passing would be,
  and costs nothing to emulate. If an interrupt is already pending, `wfi` does
  nothing.
- **And when nothing is due:** halt with an error, or sleep for ever as `wfe`
  already does when nothing can send an event _(chosen)_. QEMU and the Pi would
  both hang, and a kernel that waits for an interrupt that cannot come should look
  the same in Nebu, with a message on standard error saying why.

`wfi` at EL0 halts Nebu: NeOS never runs it there, and whether it traps depends on
SCTLR_EL1.nTWI, which Nebu does not model.

## Decision

1. **The system timer** at 0x3F00_3000 (a 0x1C-byte window) implements its status
   register (read; writing 1 bits clears them), its counter's low and high words
   (read only) and its four comparators (read and write). Its counter is the
   core's counter times 2, divided by 125. A comparator's status bit is set when
   the counter's low word passes through its value.
2. **The BCM2835 controller** at 0x3F00_B200 implements, as well as its three
   disable registers, its two pending registers (read: every raised interrupt, of
   which only comparators 0 to 3 can be), its FIQ control (write 0 only) and its
   two enable registers (write: interrupts 1 and 3 only, and nothing in the second).
   Enabling anything else halts Nebu.
3. **The local block** accepts 0 in its GPU routing register (0x0C), which sends
   the BCM2835 line to core 0's IRQ, and core 0's IRQ source shows bit 8 while an
   enabled interrupt is raised. An IRQ from it is taken between instructions, as
   the virtual timer's is (ADR 0016, decision 4).
4. **`wfi`** at EL1 does nothing if an interrupt is pending, and otherwise moves the
   core's counter to the earlier of the virtual timer's deadline and the next match
   of an enabled comparator, so that the next instruction finds it pending. With
   neither due, the core sleeps for ever, as for `wfe`. At EL0 it halts Nebu.

## Consequences

- Nebu runs all twelve of NeOS's test scenarios, including the new `interrupts`
  scenario, with the same output as QEMU.
- A wait for an interrupt takes no time to emulate, however long it is.
- The kernel's masking of the pending registers is tested in Nebu as well as QEMU.
- The next device a driver needs, such as the UART's interrupt in NeOS stage 7,
  needs its line added here first.
- Nebu implements 90 instructions, 27 system registers, six kinds of exception and
  six devices.
