//! The BCM2836 "local peripherals", which route interrupts to each core, and the
//! BCM2835 interrupt controller, which gathers the devices' interrupts into one line
//! (ADRs 0016 and 0017).
//!
//! Nebu models every core's virtual timer interrupt (ADR 0031), mailbox 0 of every
//! core as an IRQ (ADR 0050), the BCM2835 line
//! routed to core 0 as an IRQ, and on that line the system timer's comparators 1 and 3, the PL011
//! UART's interrupt, 57 (ADR 0018), the EMMC controller's, 62 (ADR 0019), and the USB
//! controller's, 9 (ADR 0024). Every other source must stay off: the local
//! timer, mailboxes 1 to 3, the FIQ, and every other device interrupt,
//! which Nebu never raises.

/// The local block's registers that NeOS uses: the local timer's control, then core
/// 0's timer interrupt control, mailbox interrupt control and pending IRQ sources,
/// each followed by those of cores 1 to 3, 4 bytes apart. Before them, which core the
/// BCM2835 line goes to, and as which kind of interrupt.
const GPU_ROUTING: u64 = 0x0C;
const LOCAL_TIMER_CONTROL: u64 = 0x34;
const CORE0_TIMER_INTERRUPTS: u64 = 0x40;
const CORE0_MAILBOX_INTERRUPTS: u64 = 0x50;
const CORE0_IRQ_SOURCE: u64 = 0x60;
/// Core 0's mailbox 0 write-set register, and its read and write-clear register; each
/// core's are 16 bytes after the last's (ADR 0050).
const CORE0_MAILBOX0_SET: u64 = 0x80;
const CORE0_MAILBOX0_CLEAR: u64 = 0xC0;
/// How many cores the local block serves.
const CORES: usize = 4;
/// The virtual timer's bit, in both the timer interrupt control and the IRQ source.
const VIRTUAL_TIMER: u32 = 1 << 3;
/// The BCM2835 line's bit in the IRQ source.
const GPU: u32 = 1 << 8;
/// Mailbox 0's bit in the mailbox interrupt control, as an IRQ.
const MAILBOX0_IRQ: u32 = 1 << 0;
/// Mailbox 0's bit in the IRQ source.
const MAILBOX0: u32 = 1 << 4;

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
    /// Each core's timer interrupt control: which of its timers may raise an IRQ.
    timer_interrupts: [u32; CORES],
    /// Each core's mailbox interrupt control: whether its mailbox 0 raises an IRQ.
    mailbox_interrupts: [u32; CORES],
    /// Each core's mailbox 0.
    mailboxes: [u32; CORES],
    /// Whether each core's virtual timer is raising its interrupt, as the CPU says.
    virtual_timer: [bool; CORES],
    /// Whether the BCM2835 line is raised.
    gpu: bool,
}

/// Which core's register `offset` is, if it is one of the four at `base`.
fn core_register(offset: u64, base: u64) -> Option<usize> {
    core_register_every(offset, base, 4)
}

/// Which core's register `offset` is, if it is one of the four at `base`, each
/// `stride` bytes after the last.
fn core_register_every(offset: u64, base: u64, stride: u64) -> Option<usize> {
    let index = offset.checked_sub(base)?;
    (index.is_multiple_of(stride) && index / stride < CORES as u64)
        .then_some((index / stride) as usize)
}

impl Local {
    /// Reads the register at `offset`, if Nebu implements it.
    pub fn read(&self, offset: u64) -> Option<u32> {
        if let Some(core) = core_register(offset, CORE0_TIMER_INTERRUPTS) {
            return Some(self.timer_interrupts[core]);
        }
        if let Some(core) = core_register(offset, CORE0_MAILBOX_INTERRUPTS) {
            return Some(self.mailbox_interrupts[core]);
        }
        if let Some(core) = core_register_every(offset, CORE0_MAILBOX0_CLEAR, 16) {
            return Some(self.mailboxes[core]);
        }
        core_register(offset, CORE0_IRQ_SOURCE).map(|core| self.pending(core))
    }

    /// Writes the register at `offset`, if Nebu implements it and the value turns on
    /// nothing but a virtual timer's interrupt or a mailbox 0's IRQ.
    pub fn write(&mut self, offset: u64, value: u32) -> Option<()> {
        if let Some(core) = core_register(offset, CORE0_TIMER_INTERRUPTS) {
            if value != 0 && value != VIRTUAL_TIMER {
                return None;
            }
            self.timer_interrupts[core] = value;
            return Some(());
        }
        if let Some(core) = core_register(offset, CORE0_MAILBOX_INTERRUPTS) {
            if value != 0 && value != MAILBOX0_IRQ {
                return None;
            }
            self.mailbox_interrupts[core] = value;
            return Some(());
        }
        if let Some(core) = core_register_every(offset, CORE0_MAILBOX0_SET, 16) {
            self.mailboxes[core] |= value;
            return Some(());
        }
        if let Some(core) = core_register_every(offset, CORE0_MAILBOX0_CLEAR, 16) {
            self.mailboxes[core] &= !value;
            return Some(());
        }
        let off = matches!(offset, GPU_ROUTING | LOCAL_TIMER_CONTROL);
        (off && value == 0).then_some(())
    }

    /// Records whether `core`'s virtual timer is raising its interrupt.
    pub fn set_virtual_timer(&mut self, core: u8, raised: bool) {
        self.virtual_timer[usize::from(core)] = raised;
    }

    /// Records whether the BCM2835 line is raised. It always goes to core 0's IRQ,
    /// the only routing Nebu accepts.
    pub fn set_gpu(&mut self, raised: bool) {
        self.gpu = raised;
    }

    /// Whether `core`'s virtual timer, once it fires, raises an IRQ.
    pub fn virtual_timer_routed(&self, core: u8) -> bool {
        self.timer_interrupts[usize::from(core)] & VIRTUAL_TIMER != 0
    }

    /// `core`'s pending IRQ sources. Only core 0 has the BCM2835 line.
    fn pending(&self, core: usize) -> u32 {
        let timer =
            if self.virtual_timer[core] { self.timer_interrupts[core] & VIRTUAL_TIMER } else { 0 };
        let mailbox =
            self.mailbox_interrupts[core] & MAILBOX0_IRQ != 0 && self.mailboxes[core] != 0;
        timer | if self.gpu && core == 0 { GPU } else { 0 } | if mailbox { MAILBOX0 } else { 0 }
    }

    /// Whether `core` has an IRQ pending.
    pub fn irq_pending(&self, core: u8) -> bool {
        self.pending(usize::from(core)) != 0
    }
}

/// The device interrupts Nebu can raise: the system timer's comparators 1 and 3, the
/// USB controller's, the PL011 UART's and the EMMC controller's. Comparators 0 and 2 are the GPU's on the
/// Pi, and NeOS never enables them.
const MODELLED: u64 = 0b1010 | USB | UART | EMMC;

/// The DWC2 USB controller's interrupt (ADR 0024).
pub const USB: u64 = 1 << 9;

/// The PL011 UART's interrupt (ADR 0018).
pub const UART: u64 = 1 << 57;
/// The Arasan EMMC controller's interrupt (ADR 0019).
pub const EMMC: u64 = 1 << 62;

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
            ENABLE_IRQS_2 if high & !MODELLED == 0 => self.enabled |= high,
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
        local.set_virtual_timer(0, true);
        assert!(!local.irq_pending(0));
        local.write(CORE0_TIMER_INTERRUPTS, VIRTUAL_TIMER).unwrap();
        assert!(local.irq_pending(0));
        assert_eq!(local.read(CORE0_IRQ_SOURCE), Some(VIRTUAL_TIMER));
        local.set_virtual_timer(0, false);
        assert_eq!(local.read(CORE0_IRQ_SOURCE), Some(0));
    }

    #[test]
    fn gives_each_core_its_own_timer_and_only_core_0_the_line() {
        let mut local = Local::default();
        local.write(CORE0_TIMER_INTERRUPTS + 8, VIRTUAL_TIMER).unwrap();
        local.write(CORE0_MAILBOX_INTERRUPTS + 12, 0).unwrap();
        local.set_virtual_timer(2, true);
        local.set_gpu(true);
        assert_eq!(local.read(CORE0_IRQ_SOURCE + 8), Some(VIRTUAL_TIMER));
        assert_eq!(local.read(CORE0_IRQ_SOURCE), Some(GPU));
        assert_eq!(local.read(CORE0_IRQ_SOURCE + 4), Some(0));
        assert!(local.irq_pending(2) && local.irq_pending(0) && !local.irq_pending(1));
        assert_eq!(local.read(CORE0_IRQ_SOURCE + 16), None);
    }

    #[test]
    fn refuses_to_turn_on_anything_else() {
        let mut local = Local::default();
        assert_eq!(local.write(CORE0_TIMER_INTERRUPTS, 1 << 1), None);
        assert_eq!(local.write(LOCAL_TIMER_CONTROL, 1 << 28), None);
        assert_eq!(local.write(CORE0_MAILBOX_INTERRUPTS, 1 << 1), None);
        assert_eq!(local.write(CORE0_MAILBOX_INTERRUPTS + 4, 1 << 4), None);
        assert_eq!(local.write(CORE0_MAILBOX0_SET + 4, 1), None);
        assert_eq!(local.write(CORE0_MAILBOX0_CLEAR + 12, 1), None);
        assert_eq!(local.read(CORE0_MAILBOX0_SET), None);
        assert_eq!(local.write(GPU_ROUTING, 4), None);
        let mut controller = Controller::default();
        assert_eq!(controller.write(ENABLE_IRQS_1, 1), None);
        assert_eq!(controller.write(ENABLE_IRQS_2, 1), None);
        assert_eq!(controller.write(FIQ_CONTROL, 0x80), None);
        assert_eq!(controller.write(DISABLE_IRQS_1, u32::MAX), Some(()));
    }

    #[test]
    fn raises_a_mailbox_only_on_its_core_and_while_enabled() {
        let mut local = Local::default();
        local.write(CORE0_MAILBOX0_SET + 2 * 16, 0b11).unwrap();
        assert_eq!(local.read(CORE0_MAILBOX0_CLEAR + 2 * 16), Some(0b11));
        assert!(!local.irq_pending(2));
        local.write(CORE0_MAILBOX_INTERRUPTS + 8, MAILBOX0_IRQ).unwrap();
        assert_eq!(local.read(CORE0_IRQ_SOURCE + 8), Some(MAILBOX0));
        assert!(local.irq_pending(2) && !local.irq_pending(0) && !local.irq_pending(3));
        local.write(CORE0_MAILBOX0_CLEAR + 2 * 16, 0b01).unwrap();
        assert_eq!(local.read(CORE0_MAILBOX0_CLEAR + 2 * 16), Some(0b10));
        assert!(local.irq_pending(2));
        local.write(CORE0_MAILBOX0_CLEAR + 2 * 16, 0b10).unwrap();
        assert!(!local.irq_pending(2));
        assert_eq!(local.read(CORE0_MAILBOX0_CLEAR + 4 * 16), None);
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
        assert!(local.irq_pending(0));
    }

    #[test]
    fn enables_and_disables_the_uart() {
        let mut controller = Controller::default();
        controller.write(ENABLE_IRQS_2, 1 << 25).unwrap();
        assert_eq!(controller.enabled(), UART);
        assert_eq!(controller.read(PENDING_2, UART), Some(1 << 25));
        assert_eq!(controller.write(ENABLE_IRQS_2, 1 << 24), None);
        controller.write(DISABLE_IRQS_2, 1 << 25).unwrap();
        assert_eq!(controller.enabled(), 0);
    }
}
