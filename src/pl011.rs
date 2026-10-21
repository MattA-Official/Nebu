//! The Arm PrimeCell PL011 UART, as QEMU's `raspi3b` models it.
//!
//! Only the registers NeOS uses are implemented (ADR 0001). Register offsets are from
//! the PrimeCell UART (PL011) Technical Reference Manual (Arm DDI 0183).
//!
//! It raises two of its interrupts, as QEMU 8.2 does (ADR 0018): receive, when a byte
//! arrives with none waiting, and transmit, whenever a byte is sent.

use std::collections::VecDeque;
use std::io::Write;
use std::sync::mpsc::Receiver;

/// Data register: written to send a byte, read to take a received one.
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
/// FIFO level select register.
const IFLS: u64 = 0x34;
/// Interrupt mask register.
const IMSC: u64 = 0x38;
/// Raw interrupt status register (read-only).
const RIS: u64 = 0x3C;
/// Masked interrupt status register (read-only).
const MIS: u64 = 0x40;
/// Interrupt clear register (write-only).
const ICR: u64 = 0x44;

/// The receive, transmit and receive timeout interrupts, the same bit in IMSC, RIS,
/// MIS and ICR. Nebu never raises the timeout, as QEMU never does, but accepts it
/// in the mask.
const INT_RX: u32 = 1 << 4;
const INT_TX: u32 = 1 << 5;
const INT_RT: u32 = 1 << 6;

/// FR: the receive FIFO is empty.
const FR_RXFE: u32 = 1 << 4;
/// FR: the transmit FIFO is empty.
const FR_TXFE: u32 = 1 << 7;

/// DR: the bits that hold the data. Nebu never reports a line error in the bits above.
const DR_DATA: u32 = 0xFF;

/// A PL011 whose transmitted bytes go to `output` and whose received bytes come from
/// `input`.
pub struct Pl011 {
    output: Box<dyn Write>,
    input: Receiver<Vec<u8>>,
    /// Bytes that have arrived from `input` but not been read yet. Unlike the real
    /// 16-byte FIFO this never fills, so nothing is ever lost.
    received: VecDeque<u8>,
    ibrd: u32,
    fbrd: u32,
    lcrh: u32,
    cr: u32,
    ifls: u32,
    imsc: u32,
    /// The interrupts raised, as RIS shows them.
    ris: u32,
}

impl Pl011 {
    /// Creates a UART in its reset state.
    pub fn new(output: Box<dyn Write>, input: Receiver<Vec<u8>>) -> Self {
        // Reset values from the TRM: TXE and RXE are set, everything else is clear.
        Pl011 {
            output,
            input,
            received: VecDeque::new(),
            ibrd: 0,
            fbrd: 0,
            lcrh: 0,
            cr: 0x300,
            ifls: 0x12,
            imsc: 0,
            ris: 0,
        }
    }

    /// Takes any bytes that have arrived from `input`. Returns whether the receive
    /// interrupt was raised by them.
    pub fn poll(&mut self) -> bool {
        let mut raised = false;
        while let Ok(bytes) = self.input.try_recv() {
            raised |= self.arrive(bytes);
        }
        raised
    }

    /// Waits for bytes to arrive from `input`. Returns false if none ever will,
    /// because the input has ended.
    pub fn wait_for_input(&mut self) -> bool {
        match self.input.recv() {
            Ok(bytes) => {
                self.arrive(bytes);
                true
            }
            Err(_) => false,
        }
    }

    /// Queues `bytes`. Like QEMU, the receive interrupt is raised when the first
    /// byte arrives with none waiting. Returns whether it was.
    fn arrive(&mut self, bytes: Vec<u8>) -> bool {
        let raised = self.received.is_empty() && !bytes.is_empty();
        self.received.extend(bytes);
        if raised {
            self.ris |= INT_RX;
        }
        raised
    }

    /// Whether a received byte is waiting to be read.
    fn has_input(&mut self) -> bool {
        self.poll();
        !self.received.is_empty()
    }

    /// Whether an interrupt that is enabled is raised.
    pub fn interrupt(&self) -> bool {
        self.ris & self.imsc != 0
    }

    /// Whether the receive interrupt is enabled, so that input would raise it.
    pub fn receive_enabled(&self) -> bool {
        self.imsc & INT_RX != 0
    }

    /// Reads the 32-bit register at `offset`, or `None` if it is not implemented.
    pub fn read(&mut self, offset: u64) -> Option<u32> {
        match offset {
            // Bytes are sent instantly, so the transmit FIFO is always empty and the
            // UART is never busy.
            FR => Some(if self.has_input() { FR_TXFE } else { FR_TXFE | FR_RXFE }),
            // Like QEMU, reading with nothing received gives 0, and reading the last
            // byte lowers the receive interrupt.
            DR => {
                let _ = self.has_input();
                let byte = self.received.pop_front().map_or(0, u32::from) & DR_DATA;
                if self.received.is_empty() {
                    self.ris &= !INT_RX;
                }
                Some(byte)
            }
            IBRD => Some(self.ibrd),
            FBRD => Some(self.fbrd),
            LCRH => Some(self.lcrh),
            CR => Some(self.cr),
            IFLS => Some(self.ifls),
            IMSC => Some(self.imsc),
            RIS => Some(self.ris),
            MIS => Some(self.ris & self.imsc),
            _ => None,
        }
    }

    /// Writes the 32-bit register at `offset`, returning `None` if it is not
    /// implemented, or if the value enables an interrupt Nebu does not model.
    pub fn write(&mut self, offset: u64, value: u32) -> Option<()> {
        match offset {
            DR => {
                // Like QEMU, transmit whether or not the UART is enabled. Real hardware
                // would drop the byte while CR.UARTEN or CR.TXE is clear.
                let byte = value as u8;
                let _ = self.output.write_all(&[byte]);
                let _ = self.output.flush();
                // The byte has gone at once, so the FIFO has drained, as QEMU says
                // after every byte.
                self.ris |= INT_TX;
            }
            ICR => self.ris &= !value,
            IBRD => self.ibrd = value & 0xFFFF,
            FBRD => self.fbrd = value & 0x3F,
            LCRH => self.lcrh = value & 0xFF,
            CR => self.cr = value & 0xFFFF,
            IFLS => self.ifls = value & 0x3F,
            IMSC if value & !(INT_RX | INT_TX | INT_RT) == 0 => self.imsc = value,
            _ => return None,
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    #[test]
    fn receives_bytes_in_order_and_reports_an_empty_fifo() {
        let (sender, receiver) = mpsc::channel();
        let mut uart = Pl011::new(Box::new(std::io::sink()), receiver);
        assert_eq!(uart.read(FR), Some(FR_TXFE | FR_RXFE));

        sender.send(vec![b'h', b'i']).unwrap();
        assert_eq!(uart.read(FR), Some(FR_TXFE));
        assert_eq!(uart.read(DR), Some(u32::from(b'h')));
        assert_eq!(uart.read(DR), Some(u32::from(b'i')));
        assert_eq!(uart.read(FR), Some(FR_TXFE | FR_RXFE));
        assert_eq!(uart.read(DR), Some(0));
    }

    #[test]
    fn raises_the_receive_interrupt_for_the_first_byte_and_lowers_it_on_the_last() {
        let (sender, receiver) = mpsc::channel();
        let mut uart = Pl011::new(Box::new(std::io::sink()), receiver);
        uart.write(IMSC, INT_RX | INT_RT).unwrap();
        assert!(!uart.poll());

        sender.send(vec![b'a', b'b']).unwrap();
        assert!(uart.poll());
        assert_eq!(uart.read(MIS), Some(INT_RX));
        assert!(uart.interrupt());
        // More input while bytes wait raises nothing new.
        uart.write(ICR, INT_RX).unwrap();
        sender.send(vec![b'c']).unwrap();
        assert!(!uart.poll());
        assert!(!uart.interrupt());
        uart.read(DR).unwrap();
        uart.read(DR).unwrap();
        uart.read(DR).unwrap();
        assert_eq!(uart.read(RIS), Some(0));
    }

    #[test]
    fn raises_the_transmit_interrupt_for_every_byte_sent() {
        let (_sender, receiver) = mpsc::channel();
        let mut uart = Pl011::new(Box::new(std::io::sink()), receiver);
        uart.write(DR, u32::from(b'x')).unwrap();
        assert_eq!(uart.read(RIS), Some(INT_TX));
        assert!(!uart.interrupt());
        uart.write(IMSC, INT_TX).unwrap();
        assert!(uart.interrupt());
        uart.write(ICR, 0x7FF).unwrap();
        assert!(!uart.interrupt());
    }

    #[test]
    fn halts_on_interrupts_it_does_not_model() {
        let (_sender, receiver) = mpsc::channel();
        let mut uart = Pl011::new(Box::new(std::io::sink()), receiver);
        // The overrun error interrupt.
        assert_eq!(uart.write(IMSC, 1 << 10), None);
    }

    #[test]
    fn wait_for_input_ends_when_input_does() {
        let (sender, receiver) = mpsc::channel();
        let mut uart = Pl011::new(Box::new(std::io::sink()), receiver);
        sender.send(vec![b'z']).unwrap();
        assert!(uart.wait_for_input());
        assert_eq!(uart.read(RIS), Some(INT_RX));
        drop(sender);
        assert!(!uart.wait_for_input());
    }
}
