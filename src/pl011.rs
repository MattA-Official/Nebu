//! The Arm PrimeCell PL011 UART, as QEMU's `raspi3b` models it.
//!
//! Only the registers NeOS uses are implemented (ADR 0001). Register offsets are from
//! the PrimeCell UART (PL011) Technical Reference Manual (Arm DDI 0183).

use std::io::Write;

/// Data register.
const DR: u64 = 0x00;
/// Flag register.
const FR: u64 = 0x18;
/// Integer baud rate divisor.
const IBRD: u64 = 0x24;
/// Fractional baud rate divisor.
const FBRD: u64 = 0x28;
/// Line control register.
const LCRH: u64 = 0x2C;
/// Control register.
const CR: u64 = 0x30;
/// Interrupt mask register.
const IMSC: u64 = 0x38;
/// Interrupt clear register (write-only).
const ICR: u64 = 0x44;

/// FR: the receive FIFO is empty.
const FR_RXFE: u32 = 1 << 4;
/// FR: the transmit FIFO is empty.
const FR_TXFE: u32 = 1 << 7;

/// A PL011 whose transmitted bytes go to `output`.
pub struct Pl011 {
    output: Box<dyn Write>,
    ibrd: u32,
    fbrd: u32,
    lcrh: u32,
    cr: u32,
    imsc: u32,
}

impl Pl011 {
    /// Creates a UART in its reset state.
    pub fn new(output: Box<dyn Write>) -> Self {
        // Reset values from the TRM: TXE and RXE are set, everything else is clear.
        Pl011 { output, ibrd: 0, fbrd: 0, lcrh: 0, cr: 0x300, imsc: 0 }
    }

    /// Reads the 32-bit register at `offset`, or `None` if it is not implemented.
    pub fn read(&mut self, offset: u64) -> Option<u32> {
        match offset {
            // Bytes are sent instantly, so the transmit FIFO is always empty. Nebu has
            // no input yet, so the receive FIFO is always empty too.
            FR => Some(FR_TXFE | FR_RXFE),
            IBRD => Some(self.ibrd),
            FBRD => Some(self.fbrd),
            LCRH => Some(self.lcrh),
            CR => Some(self.cr),
            IMSC => Some(self.imsc),
            _ => None,
        }
    }

    /// Writes the 32-bit register at `offset`, returning `None` if it is not
    /// implemented.
    pub fn write(&mut self, offset: u64, value: u32) -> Option<()> {
        match offset {
            DR => {
                // Like QEMU, transmit whether or not the UART is enabled. Real hardware
                // would drop the byte while CR.UARTEN or CR.TXE is clear.
                let byte = value as u8;
                let _ = self.output.write_all(&[byte]);
                let _ = self.output.flush();
            }
            // No interrupts are ever raised, so there is nothing to clear.
            ICR => {}
            IBRD => self.ibrd = value & 0xFFFF,
            FBRD => self.fbrd = value & 0x3F,
            LCRH => self.lcrh = value & 0xFF,
            CR => self.cr = value & 0xFFFF,
            IMSC => self.imsc = value & 0x7FF,
            _ => return None,
        }
        Some(())
    }
}
