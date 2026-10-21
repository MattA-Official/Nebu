//! The BCM2836 "local peripherals", which route interrupts to each core, and the
//! BCM2835 interrupt controller's disable registers (ADR 0016).
//!
//! Nebu models core 0's virtual timer interrupt and nothing else. Every other source
//! must stay off: the local timer, core 0's mailbox interrupts, and every device
//! interrupt the BCM2835 controller gathers, which Nebu never raises.

/// The local block's registers that NeOS uses: the local timer's control, then core
/// 0's timer interrupt control, mailbox interrupt control and pending IRQ sources.
const LOCAL_TIMER_CONTROL: u64 = 0x34;
const CORE0_TIMER_INTERRUPTS: u64 = 0x40;
const CORE0_MAILBOX_INTERRUPTS: u64 = 0x50;
const CORE0_IRQ_SOURCE: u64 = 0x60;
/// The virtual timer's bit, in both the timer interrupt control and the IRQ source.
const VIRTUAL_TIMER: u32 = 1 << 3;

/// The BCM2835 controller's disable registers, as offsets from its base at
/// 0x3F00_B200. Writing 1 bits disables those interrupts.
const DISABLE_IRQS_1: u64 = 0x1C;
const DISABLE_IRQS_2: u64 = 0x20;
const DISABLE_BASIC_IRQS: u64 = 0x24;

/// The local block's state.
#[derive(Default)]
pub struct Local {
    /// Core 0's timer interrupt control: which of its timers may raise an IRQ.
    timer_interrupts: u32,
    /// Whether core 0's virtual timer is raising its interrupt, as the CPU says.
    virtual_timer: bool,
}

impl Local {
    /// Reads the register at `offset`, if Nebu implements it.
    pub fn read(&self, offset: u64) -> Option<u32> {
        match offset {
            CORE0_TIMER_INTERRUPTS => Some(self.timer_interrupts),
            CORE0_IRQ_SOURCE => Some(self.pending()),
            _ => None,
        }
    }

    /// Writes the register at `offset`, if Nebu implements it and the value turns on
    /// nothing but the virtual timer's interrupt.
    pub fn write(&mut self, offset: u64, value: u32) -> Option<()> {
        match (offset, value) {
            (LOCAL_TIMER_CONTROL | CORE0_MAILBOX_INTERRUPTS, 0) => Some(()),
            (CORE0_TIMER_INTERRUPTS, 0 | VIRTUAL_TIMER) => {
                self.timer_interrupts = value;
                Some(())
            }
            _ => None,
        }
    }

    /// Records whether the virtual timer is raising its interrupt.
    pub fn set_virtual_timer(&mut self, raised: bool) {
        self.virtual_timer = raised;
    }

    /// Core 0's pending IRQ sources.
    fn pending(&self) -> u32 {
        if self.virtual_timer { self.timer_interrupts & VIRTUAL_TIMER } else { 0 }
    }

    /// Whether core 0 has an IRQ pending.
    pub fn irq_pending(&self) -> bool {
        self.pending() != 0
    }
}

/// Writes a BCM2835 interrupt controller register, if it is one of the disable
/// registers. Nebu raises no device interrupts, so there is nothing to disable.
pub fn write_controller(offset: u64) -> Option<()> {
    matches!(offset, DISABLE_IRQS_1 | DISABLE_IRQS_2 | DISABLE_BASIC_IRQS).then_some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raises_the_virtual_timer_only_when_enabled() {
        let mut local = Local::default();
        local.set_virtual_timer(true);
        assert!(!local.irq_pending());
        local.write(CORE0_TIMER_INTERRUPTS, VIRTUAL_TIMER).unwrap();
        assert!(local.irq_pending());
        assert_eq!(local.read(CORE0_IRQ_SOURCE), Some(VIRTUAL_TIMER));
        local.set_virtual_timer(false);
        assert_eq!(local.read(CORE0_IRQ_SOURCE), Some(0));
    }

    #[test]
    fn refuses_to_turn_on_anything_else() {
        let mut local = Local::default();
        assert_eq!(local.write(CORE0_TIMER_INTERRUPTS, 1 << 1), None);
        assert_eq!(local.write(LOCAL_TIMER_CONTROL, 1 << 28), None);
        assert_eq!(local.write(CORE0_MAILBOX_INTERRUPTS, 1), None);
        assert_eq!(write_controller(0x10), None);
        assert_eq!(write_controller(DISABLE_IRQS_1), Some(()));
    }
}
