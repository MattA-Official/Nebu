//! The BCM2835's hardware random number generator, as far as NeOS uses it
//! (Nebu ADR 0032).
//!
//! Broadcom does not document it, and what is inside it is not public, so there is
//! nothing to copy faithfully: what matters is that the registers behave as NeOS's
//! random service expects, and that the words it reads are different on every run.
//! The numbers come from a generator of Nebu's own, seeded from the host's clock,
//! not from the host's randomness, so a run can be repeated by setting the seed.
//!
//! Register offsets are the ones Linux's `bcm2835-rng` driver uses.

use std::time::{SystemTime, UNIX_EPOCH};

/// Bit 0 starts the generator.
const CONTROL: u64 = 0x0;
/// Bits 31 to 24 count the words waiting; the low 20 bits are the warm-up count.
const STATUS: u64 = 0x4;
/// Reading this takes the next word.
const DATA: u64 = 0x8;
/// Bit 0 masks the generator's interrupt.
const INTERRUPT_MASK: u64 = 0x10;

/// The words the status register says are waiting. The real generator fills a queue
/// as fast as it can; Nebu always has one ready, as QEMU's model does, so a driver
/// that waits for one never waits long.
const WAITING: u32 = 1 << 24;

/// The generator's registers, and the numbers it gives.
pub struct Rng {
    control: u32,
    /// The warm-up count, the only part of the status register that is written.
    warm_up: u32,
    interrupt_mask: u32,
    /// The state of the generator behind the data register.
    state: u64,
}

impl Default for Rng {
    fn default() -> Rng {
        // The clock is the only thing to hand that differs between runs. Nebu models
        // no clock of its own, so this is the host's.
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| since.as_nanos() as u64)
            .unwrap_or(0x243F_6A88_85A3_08D3);
        Rng { control: 0, warm_up: 0, interrupt_mask: 0, state: seed | 1 }
    }
}

impl Rng {
    /// Reads the 32-bit register at `offset`, or `None` if it is not implemented.
    pub fn read(&mut self, offset: u64) -> Option<u32> {
        match offset {
            CONTROL => Some(self.control),
            STATUS => Some(self.warm_up | WAITING),
            // A word is given whether or not the generator was started: a driver
            // that forgets to start it is not what Nebu is there to catch, and the
            // real generator's queue may hold words from before it was stopped.
            DATA => Some(self.word()),
            INTERRUPT_MASK => Some(self.interrupt_mask),
            _ => None,
        }
    }

    /// Writes the 32-bit register at `offset`, or `None` if it is not implemented.
    pub fn write(&mut self, offset: u64, value: u32) -> Option<()> {
        match offset {
            CONTROL => self.control = value,
            // Only the warm-up count can be written; the count of words waiting is
            // the hardware's to say.
            STATUS => self.warm_up = value & 0xF_FFFF,
            INTERRUPT_MASK => self.interrupt_mask = value,
            _ => return None,
        }
        Some(())
    }

    /// The next word: SplitMix64, whose output is spread evenly and which is short
    /// enough to read in one sitting. It is not meant to stand up to anyone trying
    /// to guess it, and NeOS mixes what it gives with its own generator anyway.
    fn word(&mut self) -> u32 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut word = self.state;
        word = (word ^ (word >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        word = (word ^ (word >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((word ^ (word >> 31)) >> 32) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::{CONTROL, DATA, INTERRUPT_MASK, Rng, STATUS, WAITING};

    /// The status register always has a word waiting, and keeps the warm-up count
    /// that was written, not the count of words.
    #[test]
    fn status_says_a_word_is_waiting() {
        let mut rng = Rng::default();
        assert_eq!(rng.write(STATUS, 0x4_0000), Some(()));
        assert_eq!(rng.read(STATUS), Some(0x4_0000 | WAITING));
        assert_eq!(super::Rng::default().read(STATUS), Some(WAITING));
    }

    /// Control and the interrupt mask read back what was written.
    #[test]
    fn registers_read_back() {
        let mut rng = Rng::default();
        rng.write(CONTROL, 1);
        rng.write(INTERRUPT_MASK, 1);
        assert_eq!(rng.read(CONTROL), Some(1));
        assert_eq!(rng.read(INTERRUPT_MASK), Some(1));
        assert_eq!(rng.read(0xC), None);
        assert_eq!(rng.write(0xC, 0), None);
    }

    /// Each word differs from the one before, and the spread over the bytes is even.
    #[test]
    fn words_differ_and_are_spread() {
        let mut rng = Rng::default();
        let first = rng.read(DATA);
        assert_ne!(first, rng.read(DATA));
        let mut counts = [0_u32; 256];
        for _ in 0..4096 {
            let word = rng.read(DATA).expect("the data register is implemented");
            for byte in word.to_le_bytes() {
                counts[usize::from(byte)] += 1;
            }
        }
        // 16,384 bytes over 256 values: 64 each on average.
        for count in counts {
            assert!(count.abs_diff(64) < 40, "a byte value came up {count} times");
        }
    }

    /// Two generators made a moment apart give different words.
    #[test]
    fn runs_differ() {
        let mut first = Rng::default();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let mut second = Rng::default();
        assert_ne!(first.read(DATA), second.read(DATA));
    }
}
