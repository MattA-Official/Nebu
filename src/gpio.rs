//! The BCM2837's GPIO controller, as far as NeOS uses it: routing pins 14 and 15 to the
//! PL011, and pins 48 to 53, the SD card's, to the Arasan EMMC controller (ADR 0019).
//!
//! Nebu has no pins, so the registers only store what is written. Register offsets are
//! from the BCM2835 ARM Peripherals manual, section 6.

/// Function select for pins 10 to 19, 30 to 39, 40 to 49 and 50 to 59.
const GPFSEL1: u64 = 0x04;
const GPFSEL3: u64 = 0x0C;
const GPFSEL4: u64 = 0x10;
const GPFSEL5: u64 = 0x14;
/// Alternate function 3, which connects pins 48 to 53 to the Arasan controller.
const ALT3: u32 = 0b111;
/// Pull-up/down control.
const GPPUD: u64 = 0x94;
/// Pull-up/down clock for pins 0 to 31.
const GPPUDCLK0: u64 = 0x98;

/// The GPIO registers NeOS uses.
#[derive(Default)]
pub struct Gpio {
    gpfsel1: u32,
    gpfsel3: u32,
    gpfsel4: u32,
    gpfsel5: u32,
    gppud: u32,
    gppudclk0: u32,
}

impl Gpio {
    /// Reads the 32-bit register at `offset`, or `None` if it is not implemented.
    pub fn read(&self, offset: u64) -> Option<u32> {
        match offset {
            GPFSEL1 => Some(self.gpfsel1),
            GPFSEL3 => Some(self.gpfsel3),
            GPFSEL4 => Some(self.gpfsel4),
            GPFSEL5 => Some(self.gpfsel5),
            GPPUD => Some(self.gppud),
            GPPUDCLK0 => Some(self.gppudclk0),
            _ => None,
        }
    }

    /// Writes the 32-bit register at `offset`, returning `None` if it is not
    /// implemented.
    pub fn write(&mut self, offset: u64, value: u32) -> Option<()> {
        match offset {
            // Bits 30 and 31 are reserved; pins 10 to 19 use 3 bits each.
            GPFSEL1 => self.gpfsel1 = value & 0x3FFF_FFFF,
            GPFSEL3 => self.gpfsel3 = value & 0x3FFF_FFFF,
            GPFSEL4 => self.gpfsel4 = value & 0x3FFF_FFFF,
            // Only pins 50 to 53 exist.
            GPFSEL5 => self.gpfsel5 = value & 0xFFF,
            GPPUD => self.gppud = value & 0b11,
            GPPUDCLK0 => self.gppudclk0 = value,
            _ => return None,
        }
        Some(())
    }

    /// Whether pins 48 to 53 are all set to alternate function 3, which connects the
    /// SD card to the Arasan controller.
    pub fn sd_card_connected(&self) -> bool {
        let pins_48_49 = (8..10).all(|pin| self.gpfsel4 >> (3 * pin) & 0b111 == ALT3);
        let pins_50_53 = (0..4).all(|pin| self.gpfsel5 >> (3 * pin) & 0b111 == ALT3);
        pins_48_49 && pins_50_53
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connects_the_card_only_when_all_six_pins_are_alt3() {
        let mut gpio = Gpio::default();
        assert!(!gpio.sd_card_connected());
        gpio.write(GPFSEL4, 0b111_111 << 24).unwrap();
        assert!(!gpio.sd_card_connected());
        gpio.write(GPFSEL5, 0b111_111_111_111).unwrap();
        assert!(gpio.sd_card_connected());
        gpio.write(GPFSEL5, 0b111_111_111_100).unwrap();
        assert!(!gpio.sd_card_connected());
    }
}
