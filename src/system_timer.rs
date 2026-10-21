//! The BCM2835 system timer: a 64-bit counter at 1 MHz, and four 32-bit comparators
//! that each raise an interrupt when the counter's low word equals them (ADR 0017).
//!
//! The counter follows the core's own counter, which counts one for each instruction
//! at 62.5 MHz (ADR 0016), so it counts one for every 62.5 instructions. Time in Nebu
//! is still the instructions it has run, and a run is still repeatable.

/// The registers: the status (write 1 to clear a comparator's match), the counter's
/// low and high words, and the four comparators.
const STATUS: u64 = 0x00;
const COUNTER_LOW: u64 = 0x04;
const COUNTER_HIGH: u64 = 0x08;
const COMPARATORS: u64 = 0x0C;

/// The core's counter's rate over the system timer's: 62.5 MHz over 1 MHz.
const TICKS_PER_MICROSECOND: (u64, u64) = (125, 2);

/// The system timer's state.
#[derive(Default)]
pub struct SystemTimer {
    /// Which comparators have matched since their bit was last cleared.
    status: u32,
    compare: [u32; 4],
    /// The counter, as of the last time Nebu looked.
    now: u64,
}

impl SystemTimer {
    /// Reads the register at `offset`, if Nebu implements it.
    pub fn read(&self, offset: u64) -> Option<u32> {
        match offset {
            STATUS => Some(self.status),
            COUNTER_LOW => Some(self.now as u32),
            COUNTER_HIGH => Some((self.now >> 32) as u32),
            _ => self.comparator(offset).map(|n| self.compare[n]),
        }
    }

    /// Writes the register at `offset`, if Nebu implements it. The counter cannot be
    /// written, as on hardware.
    pub fn write(&mut self, offset: u64, value: u32) -> Option<()> {
        match offset {
            STATUS => self.status &= !(value & 0xF),
            _ => self.compare[self.comparator(offset)?] = value,
        }
        Some(())
    }

    /// Which comparator's register is at `offset`, if any.
    fn comparator(&self, offset: u64) -> Option<usize> {
        let index = offset.checked_sub(COMPARATORS)?;
        (index.is_multiple_of(4) && index < 16).then_some(index as usize / 4)
    }

    /// Moves the counter on to where it is when the core's counter reads `ticks`,
    /// and marks every comparator whose value the low word passed through.
    pub fn advance(&mut self, ticks: u64) {
        let now = ticks * TICKS_PER_MICROSECOND.1 / TICKS_PER_MICROSECOND.0;
        if now == self.now {
            return;
        }
        let passed = now - self.now;
        for (n, &compare) in self.compare.iter().enumerate() {
            let ahead = u64::from(compare.wrapping_sub(self.now as u32));
            if passed > u64::from(u32::MAX) || (1..=passed).contains(&ahead) {
                self.status |= 1 << n;
            }
        }
        self.now = now;
    }

    /// Which comparators have matched: the levels of interrupts 0 to 3.
    pub fn levels(&self) -> u32 {
        self.status
    }

    /// The core's counter at which the first of the comparators in `mask` next
    /// matches.
    pub fn next_match(&self, mask: u32) -> Option<u64> {
        let micros = (0..4)
            .filter(|n| mask & 1 << n != 0)
            .map(|n| {
                // A comparator equal to the counter now matches only once the low
                // word comes round again.
                let ahead = self.compare[n].wrapping_sub(self.now as u32);
                self.now + if ahead == 0 { 1 << 32 } else { u64::from(ahead) }
            })
            .min()?;
        // The first count of the core's counter at which `advance` reaches it.
        Some((micros * TICKS_PER_MICROSECOND.0).div_ceil(TICKS_PER_MICROSECOND.1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_at_one_megahertz() {
        let mut timer = SystemTimer::default();
        timer.advance(124);
        assert_eq!(timer.read(COUNTER_LOW), Some(1));
        timer.advance(62_500_000);
        assert_eq!(timer.read(COUNTER_LOW), Some(1_000_000));
        assert_eq!(timer.read(COUNTER_HIGH), Some(0));
    }

    #[test]
    fn matches_when_the_counter_passes_a_comparator() {
        let mut timer = SystemTimer::default();
        timer.write(COMPARATORS + 4, 10).unwrap();
        timer.advance(9 * 125 / 2);
        assert_eq!(timer.levels(), 0);
        assert_eq!(timer.next_match(0b10), Some(625));
        timer.advance(625);
        assert_eq!(timer.levels(), 0b10);
        timer.write(STATUS, 0b10).unwrap();
        assert_eq!(timer.levels(), 0);
        // It does not match again until the low word wraps round.
        timer.advance(10_000);
        assert_eq!(timer.levels(), 0);
    }

    #[test]
    fn refuses_registers_it_does_not_have() {
        let mut timer = SystemTimer::default();
        assert_eq!(timer.read(0x1C), None);
        assert_eq!(timer.write(COUNTER_LOW, 0), None);
        assert_eq!(timer.write(0x0E, 0), None);
    }
}
