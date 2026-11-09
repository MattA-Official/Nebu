//! The BCM2835 power manager's watchdog, as far as NeOS uses it (Nebu ADR 0036).
//!
//! Writing a count to `WDOG` and "full reset" to `RSTC` arms the watchdog; when the
//! count runs out, the whole machine resets. Rewriting `WDOG` while it is armed starts
//! the count again, writing "full reset" to `RSTC` again does not, and clearing
//! `RSTC`'s configuration disarms it. Every write carries the password 0x5A in its top
//! byte. Writing partition 63 into `RSTS` before the reset halts the machine, as the
//! firmware stays halted (Nebu ADR 0045). Reading `WDOG`, which on the chip gives the ticks left, is not modelled, as
//! NeOS does not do it, so it halts Nebu.
//!
//! Register offsets and values are the ones Linux's `bcm2835_wdt` driver uses.

/// The reset control register: bits 5:4 say what happens when the count runs out.
const RSTC: u64 = 0x1C;
/// The reset status register.
const RSTS: u64 = 0x20;
/// The watchdog's count, in its low 20 bits.
const WDOG: u64 = 0x24;

/// What every write carries in its top byte.
const PASSWORD: u32 = 0x5A00_0000;
const PASSWORD_MASK: u32 = 0xFF00_0000;
/// RSTC's configuration bits, and the value that arms a full reset.
const WRCFG_MASK: u32 = 0x30;
const WRCFG_FULL_RESET: u32 = 0x20;
/// WDOG's count.
const TIME_MASK: u32 = 0xF_FFFF;
/// RSTS after a power-on, as QEMU has it, and the bit the hardware sets after a
/// watchdog reset.
const RSTS_POWER_ON: u32 = 0x1000;
const RSTS_WATCHDOG: u32 = 0x20;
/// RSTS's partition bits, the even bits 0 to 10, and partition 63 in them, which
/// tells the firmware to stay halted after the reset, as Linux's `bcm2835_wdt` uses
/// it to power off.
const PARTITION_BITS: u32 = 0x555;

/// The core's counts in one tick of the watchdog: the counter runs at 62.5 MHz (ADR
/// 0016) and the watchdog at 65,536 Hz.
pub const COUNTS_PER_TICK: u64 = 954;

/// Why a write was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// A register Nebu does not model.
    Register,
    /// The write did not carry the password.
    Password,
    /// A configuration of RSTC's bits 5:4 other than clear and full reset.
    Configuration,
}

/// The power manager's registers.
pub struct Power {
    rstc: u32,
    rsts: u32,
    wdog: u32,
    /// The counter at which the machine resets, while the watchdog is armed.
    deadline: Option<u64>,
}

impl Power {
    /// The registers after a power-on, or, if `after_watchdog`, after the watchdog
    /// reset the machine.
    pub fn new(after_watchdog: bool) -> Power {
        let rsts = if after_watchdog { RSTS_POWER_ON | RSTS_WATCHDOG } else { RSTS_POWER_ON };
        Power { rstc: 0x102, rsts, wdog: 0, deadline: None }
    }

    /// Reads the 32-bit register at `offset`, or `None` if it is not implemented.
    pub fn read(&self, offset: u64) -> Option<u32> {
        match offset {
            RSTC => Some(self.rstc),
            RSTS => Some(self.rsts),
            _ => None,
        }
    }

    /// Writes the 32-bit register at `offset`, with the counter at `now`.
    pub fn write(&mut self, offset: u64, value: u32, now: u64) -> Result<(), Refusal> {
        if !matches!(offset, RSTC | RSTS | WDOG) {
            return Err(Refusal::Register);
        }
        if value & PASSWORD_MASK != PASSWORD {
            return Err(Refusal::Password);
        }
        let value = value & !PASSWORD_MASK;
        let was_armed = self.armed();
        match offset {
            RSTC => {
                if !matches!(value & WRCFG_MASK, 0 | WRCFG_FULL_RESET) {
                    return Err(Refusal::Configuration);
                }
                self.rstc = value;
            }
            RSTS => self.rsts = value,
            _ => self.wdog = value & TIME_MASK,
        }
        // The count starts when the watchdog is armed, or when it is rewritten; arming
        // it again while it is armed leaves the count running.
        if !self.armed() {
            self.deadline = None;
        } else if offset == WDOG || !was_armed {
            self.deadline = Some(now + u64::from(self.wdog) * COUNTS_PER_TICK);
        }
        Ok(())
    }

    /// Whether a count that runs out resets the machine.
    fn armed(&self) -> bool {
        self.rstc & WRCFG_MASK == WRCFG_FULL_RESET && self.wdog != 0
    }

    /// The counter at which the watchdog resets the machine, if it is armed.
    pub fn deadline(&self) -> Option<u64> {
        self.deadline
    }

    /// Whether the firmware would stay halted after a reset now: `RSTS` names
    /// partition 63.
    pub fn halts(&self) -> bool {
        self.rsts & PARTITION_BITS == PARTITION_BITS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARM: u32 = PASSWORD | WRCFG_FULL_RESET;

    #[test]
    fn arming_sets_a_deadline_and_writing_the_count_again_moves_it() {
        let mut power = Power::new(false);
        power.write(WDOG, PASSWORD | 10, 0).unwrap();
        assert_eq!(power.deadline(), None);
        power.write(RSTC, ARM, 100).unwrap();
        assert_eq!(power.deadline(), Some(100 + 10 * COUNTS_PER_TICK));
        power.write(WDOG, PASSWORD | 20, 500).unwrap();
        assert_eq!(power.deadline(), Some(500 + 20 * COUNTS_PER_TICK));
        // Arming it again while it is armed leaves the count running.
        power.write(RSTC, ARM, 900).unwrap();
        assert_eq!(power.deadline(), Some(500 + 20 * COUNTS_PER_TICK));
        assert_eq!(power.read(WDOG), None);
    }

    #[test]
    fn disarming_clears_the_deadline() {
        let mut power = Power::new(false);
        power.write(WDOG, PASSWORD | 10, 0).unwrap();
        power.write(RSTC, ARM, 0).unwrap();
        power.write(RSTC, PASSWORD | 0x102, 5).unwrap();
        assert_eq!(power.deadline(), None);
    }

    #[test]
    fn a_count_of_zero_never_resets() {
        let mut power = Power::new(false);
        power.write(RSTC, ARM, 0).unwrap();
        assert_eq!(power.deadline(), None);
    }

    #[test]
    fn writes_without_the_password_or_elsewhere_are_refused() {
        let mut power = Power::new(false);
        assert_eq!(power.write(WDOG, 10, 0), Err(Refusal::Password));
        assert_eq!(power.write(RSTC, WRCFG_FULL_RESET, 0), Err(Refusal::Password));
        assert_eq!(power.write(0x10, PASSWORD, 0), Err(Refusal::Register));
        assert_eq!(power.write(RSTC, PASSWORD | 0x10, 0), Err(Refusal::Configuration));
        assert_eq!(power.read(0x10), None);
    }

    #[test]
    fn partition_63_in_the_status_halts_and_others_do_not() {
        let mut power = Power::new(true);
        assert!(!power.halts());
        assert_eq!(power.write(RSTS, 0x555, 0), Err(Refusal::Password));
        power.write(RSTS, PASSWORD | RSTS_POWER_ON | 0x554, 0).unwrap();
        assert!(!power.halts());
        // Linux's way: what it holds, with the partition bits all set.
        let status = power.read(RSTS).unwrap();
        power.write(RSTS, PASSWORD | status | PARTITION_BITS, 0).unwrap();
        assert!(power.halts());
        assert_eq!(power.read(RSTS), Some(RSTS_POWER_ON | 0x555));
        // Writing it is not arming: nothing happens until the watchdog runs out.
        assert_eq!(power.deadline(), None);
    }

    #[test]
    fn the_status_says_whether_the_watchdog_reset_the_machine() {
        assert_eq!(Power::new(false).read(RSTS), Some(RSTS_POWER_ON));
        assert_eq!(Power::new(true).read(RSTS), Some(RSTS_POWER_ON | RSTS_WATCHDOG));
    }
}
