//! The BCM2836 "local peripherals", which route interrupts to each core, and the
//! BCM2835 interrupt controller, which gathers the devices' interrupts into one line
//! (ADRs 0016 and 0017).
//!
//! Nebu models core 0's virtual timer interrupt, the BCM2835 line routed to core 0
//! as an IRQ, and on that line the system timer's comparators 1 and 3. Every other
//! source must stay off: the local timer, core 0's mailbox interrupts, the FIQ, and
//! every other device interrupt, which Nebu never raises.

/// The local block's registers that NeOS uses: the local timer's control, then core
/// 0's timer interrupt control, mailbox interrupt control and pending IRQ sources.
/// Before them, which core the BCM2835 line goes to, and as which kind of interrupt.
const GPU_ROUTING: u64 = 0x0C;
const LOCAL_TIMER_CONTROL: u64 = 0x34;
const CORE0_TIMER_INTERRUPTS: u64 = 0x40;
const CORE0_MAILBOX_INTERRUPTS: u64 = 0x50;
const CORE0_IRQ_SOURCE: u64 = 0x60;
/// The virtual timer's bit, in both the timer interrupt control and the IRQ source.
const VIRTUAL_TIMER: u32 = 1 << 3;
/// The BCM2835 line's bit in the IRQ source.
const GPU: u32 = 1 << 8;

/// The BCM2835 controller's registers, as offsets from its base at 0x3F00_B200:
/// interrupts 0 to 31 and 32 to 63 that are raised, the FIQ's control, then
/// writing 1 bits enables or disables those interrupts.
const PENDING_1: u64 = 0x04;
const PENDING_2: u64 = 0x08;
const FIQ_CONTROL: u64 = 0x0C;
const ENABLE_IRQS_1: u64 = 0x10;
const ENABLE_IRQS_2: u64 = 0x14;
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
    /// Whether the BCM2835 line is raised.
    gpu: bool,
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
            (GPU_ROUTING | LOCAL_TIMER_CONTROL | CORE0_MAILBOX_INTERRUPTS, 0) => Some(()),
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

    /// Records whether the BCM2835 line is raised. It always goes to core 0's IRQ,
    /// the only routing Nebu accepts.
    pub fn set_gpu(&mut self, raised: bool) {
        self.gpu = raised;
    }

    /// Whether core 0's virtual timer, once it fires, raises an IRQ.
    pub fn virtual_timer_routed(&self) -> bool {
        self.timer_interrupts & VIRTUAL_TIMER != 0
    }

    /// Core 0's pending IRQ sources.
    fn pending(&self) -> u32 {
        let timer = if self.virtual_timer { self.timer_interrupts & VIRTUAL_TIMER } else { 0 };
        timer | if self.gpu { GPU } else { 0 }
    }

    /// Whether core 0 has an IRQ pending.
    pub fn irq_pending(&self) -> bool {
        self.pending() != 0
    }
}

/// The device interrupts Nebu can raise: the system timer's comparators 1 and 3.
/// Comparators 0 and 2 are the GPU's on the Pi, and NeOS never enables them.
const MODELLED: u64 = 0b1010;

/// The BCM2835 controller's state.
#[derive(Default)]
pub struct Controller {
    /// Which interrupts are enabled.
    enabled: u64,
}

impl Controller {
    /// Reads the register at `offset`, if Nebu implements it. `levels` are the
    /// device interrupts raised. The pending registers show them whether or not
    /// they are enabled, as QEMU's do.
    pub fn read(&self, offset: u64, levels: u64) -> Option<u32> {
        match offset {
            PENDING_1 => Some(levels as u32),
            PENDING_2 => Some((levels >> 32) as u32),
            _ => None,
        }
    }

    /// Writes the register at `offset`, if Nebu implements it and the value turns
    /// on nothing but the interrupts it models, and no FIQ.
    pub fn write(&mut self, offset: u64, value: u32) -> Option<()> {
        let high = u64::from(value) << 32;
        match offset {
            ENABLE_IRQS_1 if u64::from(value) & !MODELLED == 0 => {
                self.enabled |= u64::from(value);
            }
            ENABLE_IRQS_2 if value == 0 => {}
            DISABLE_IRQS_1 => self.enabled &= !u64::from(value),
            DISABLE_IRQS_2 => self.enabled &= !high,
            DISABLE_BASIC_IRQS => {}
            FIQ_CONTROL if value == 0 => {}
            _ => return None,
        }
        Some(())
    }

    /// Which interrupts are enabled.
    pub fn enabled(&self) -> u64 {
        self.enabled
    }
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
        assert_eq!(local.write(GPU_ROUTING, 4), None);
        let mut controller = Controller::default();
        assert_eq!(controller.write(ENABLE_IRQS_1, 1), None);
        assert_eq!(controller.write(ENABLE_IRQS_2, 1), None);
        assert_eq!(controller.write(FIQ_CONTROL, 0x80), None);
        assert_eq!(controller.write(DISABLE_IRQS_1, u32::MAX), Some(()));
    }

    #[test]
    fn enables_and_disables_the_system_timer() {
        let mut controller = Controller::default();
        controller.write(ENABLE_IRQS_1, 0b10).unwrap();
        assert_eq!(controller.enabled(), 0b10);
        assert_eq!(controller.read(PENDING_1, 0b11), Some(0b11));
        controller.write(DISABLE_IRQS_1, 0b10).unwrap();
        assert_eq!(controller.enabled(), 0);
        let mut local = Local::default();
        local.set_gpu(true);
        assert_eq!(local.read(CORE0_IRQ_SOURCE), Some(GPU));
        assert!(local.irq_pending());
    }
}
