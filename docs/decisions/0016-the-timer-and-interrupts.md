# 0016. The timer and interrupts

- **Status:** Accepted
- **Date:** 2026-10-21
- **Amends:** [0006](0006-machine-model.md) (its list of devices) and
  [0008](0008-exception-levels-and-exceptions.md) (decision 5's rule that every
  exception it does not list halts Nebu)

## Context

NeOS stage 6a preempts threads with the generic timer and schedules them by
priority
([NeOS ADR 0021](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0021-the-timer-preemption-and-priorities.md),
proposed in NeOS #16). The kernel now:

- reads the counter's rate from CNTFRQ_EL0 and the counter from CNTVCT_EL0;
- sets CNTKCTL_EL1 to 0, so EL0 cannot reach the counter or the timers;
- arms the virtual timer as a one-shot, writing a deadline to CNTV_CVAL_EL0 and 1
  (enabled, not masked) to CNTV_CTL_EL0, before every return to user space;
- turns off every interrupt source it does not use: the BCM2835 interrupt
  controller's three disable registers at 0x3F00_B21C, 0x3F00_B220 and 0x3F00_B224,
  and, in the BCM2836 local block at 0x4000_0000, the local timer (0x34) and core
  0's mailbox interrupts (0x50);
- routes core 0's virtual timer to its IRQ by setting bit 3 of core 0's timer
  interrupt control (0x40), and reads core 0's pending sources (0x60) to check that
  the timer is the only one;
- runs user space with IRQs unmasked, and takes them at the vector 0x480 for an
  IRQ from EL0. The kernel itself always runs with them masked.

Nebu halts on all of this today: it has no timers, no local block, and takes no
interrupts.

## Options

### How the counter advances

The counter must move forward, and preemption only works if a thread that spins
reaches its deadline.

- **From the host's clock.** The counter would follow real time, as on hardware.
  But how many instructions a slice holds would depend on how fast the host is and
  how busy it is, so a run could not be repeated, and a slow CI runner would see
  different interleavings from a fast laptop. Nebu has been deterministic until now
  (ADR 0004), and its tests compare its output with QEMU's line by line.
- **One count for each instruction** _(chosen)_. Every run of the same image is
  the same, instruction for instruction, which is what makes a failure in Nebu easy
  to repeat. Time in Nebu is not real time, but nothing in NeOS needs it to be: the
  kernel only asks whether a deadline has passed.
- **Several counts for each instruction, or a count every few instructions.** This
  is also deterministic, and would let Nebu pick a ratio near the Pi's. But the Pi's
  ratio depends on caches and on which instructions run, so no fixed number matches
  it, and any number other than one is a choice with nothing to back it.

### What CNTFRQ_EL0 says

- **19.2 MHz, the Pi 3's crystal.** This would match the hardware, but not QEMU,
  which Nebu checks itself against.
- **62.5 MHz, as QEMU's `raspi3b` reports** _(chosen)_. The kernel's slices then
  hold the same number of counts as they do in QEMU. At one count per instruction,
  a 10 ms slice is 625,000 instructions in Nebu.

### The registers

- **Model the whole generic timer**: the physical timer, the hypervisor timer,
  CNTKCTL_EL1's bits for EL0 access and events, and the event stream. This is far
  more than NeOS uses, and each part would go untested (ADR 0001).
- **Model what NeOS uses** _(chosen)_. CNTFRQ_EL0 reads the rate and cannot be
  written: on the Pi only the firmware sets it. CNTVCT_EL0 reads the counter less
  CNTVOFF_EL2, which NeOS sets to 0 when it drops to EL1. CNTV_CVAL_EL0 is stored.
  CNTV_CTL_EL0 stores ENABLE and IMASK, and reads ISTATUS as set when the timer is
  enabled and the counter has reached the comparator. CNTKCTL_EL1 accepts only 0,
  the value NeOS writes, because any other value gives EL0 the counter or raises
  events, which Nebu does not model. All of them need EL1 or above, as CNTKCTL_EL1
  = 0 makes them on hardware.

### The interrupt controllers

The virtual timer's interrupt reaches the core through the local block, which can
also route the local timer, mailboxes and the BCM2835 controller's device
interrupts.

- **Model the local block and the BCM2835 controller in full.** Nebu raises no
  other interrupt, so nearly all of it would sit unused.
- **Model only core 0's virtual timer route** _(chosen)_. Core 0's timer interrupt
  control may hold 0 or bit 3, and its pending sources read bit 3 while the timer
  is firing and routed. The local timer's control and core 0's mailbox interrupt
  control accept only 0, and the BCM2835 disable registers accept any write, since
  there is nothing to disable. Writing anything that turns on another source halts
  Nebu, so a kernel that starts relying on one will find out.

### When an IRQ is taken

- **Between instructions** _(chosen)_. Before each instruction Nebu checks whether
  the timer is firing and routed, and takes the IRQ if PSTATE.I is clear. ELR_EL1
  holds the address of the instruction that has not yet run, and SPSR_EL1 the
  state before the interrupt, as the architecture says. ESR_EL1 is not written.
- **Only at certain instructions**, such as branches. It is cheaper to check, but
  the architecture lets an IRQ be taken between any two instructions, and a kernel
  bug that only shows when one lands in a particular place would be hidden.

## Decision

1. **The counter counts one for each instruction Nebu runs**, and CNTFRQ_EL0 reads
   62,500,000. Writing CNTFRQ_EL0 halts Nebu.
2. **CNTVCT_EL0, CNTV_CTL_EL0, CNTV_CVAL_EL0 and CNTKCTL_EL1** are implemented as
   above. CNTVCT_EL0 is read-only, and CNTKCTL_EL1 accepts only 0. CNTHCTL_EL2 is
   still only stored.
3. **The local block** at 0x4000_0000 (a 0x100-byte window) implements the local
   timer's control, and core 0's timer interrupt control, mailbox interrupt
   control and pending IRQ sources, accepting only the values that turn on nothing
   but the virtual timer's IRQ. **The BCM2835 interrupt controller** at 0x3F00_B200
   implements only its three disable registers.
4. **The virtual timer's IRQ is taken between instructions** when it is enabled,
   not masked by IMASK, routed by core 0's timer interrupt control, and PSTATE.I is
   clear. It uses the IRQ vectors, 0x80 after the synchronous ones: 0x480 from EL0,
   0x280 from EL1 using SP_EL1, and 0x080 from EL1 using SP_EL0. An IRQ at EL2
   halts Nebu.

## Consequences

- Nebu runs all eleven of NeOS's test scenarios, including the new `scheduling`
  scenario, with the same output as QEMU.
- A run in Nebu is repeatable: the same image preempts its threads at the same
  instructions every time. QEMU, whose timer follows the host's clock, and the Pi
  are where interleavings vary.
- Slices in Nebu hold 625,000 instructions. On the Pi they hold far more, so a test
  that only passes when a slice is short could pass in Nebu and fail on the Pi.
- Nebu implements 89 instructions, 27 system registers, six kinds of exception and
  five devices.
