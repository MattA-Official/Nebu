//! The BCM2837's GPIO controller, as far as NeOS uses it: routing pins 14 and 15 to the
//! PL011.
//!
//! Nebu has no pins, so the registers only store what is written. Register offsets are
//! from the BCM2835 ARM Peripherals manual, section 6.

/// Function select for pins 10 to 19.
const GPFSEL1: u64 = 0x04;
/// Pull-up/down control.
const GPPUD: u64 = 0x94;
/// Pull-up/down clock for pins 0 to 31.
const GPPUDCLK0: u64 = 0x98;

/// The GPIO registers NeOS uses.
#[derive(Default)]
pub struct Gpio {
    gpfsel1: u32,
    gppud: u32,
    gppudclk0: u32,
}

impl Gpio {
    /// Reads the 32-bit register at `offset`, or `None` if it is not implemented.
    pub fn read(&self, offset: u64) -> Option<u32> {
        match offset {
            GPFSEL1 => Some(self.gpfsel1),
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
            GPPUD => self.gppud = value & 0b11,
            GPPUDCLK0 => self.gppudclk0 = value,
            _ => return None,
        }
        Some(())
    }
}
