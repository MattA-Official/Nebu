//! The physical address space: RAM and memory-mapped devices.
//!
//! Every address Nebu does not model is an error, never silently ignored (ADR 0001).

use std::collections::HashMap;
use std::fmt;

use crate::pl011::Pl011;

/// RAM occupies the bottom of the address space, up to the peripherals.
const RAM_END: u64 = 0x3F00_0000;
/// The PL011 UART (UART0).
const UART0_BASE: u64 = 0x3F20_1000;
/// The size of one device's register window.
const DEVICE_WINDOW: u64 = 0x1000;

/// RAM is allocated in pages of this size, the first time each page is written.
const PAGE_SIZE: u64 = 0x1_0000;

/// What went wrong with a memory access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusError {
    /// Nothing is mapped at this address.
    Unmapped,
    /// The address is not a multiple of the access size. With the MMU off, every data
    /// access is to Device memory, which must be aligned.
    Unaligned,
    /// A device register Nebu does not implement, or an access of the wrong width.
    DeviceRegister { device: &'static str, offset: u64 },
    /// Instructions can only be fetched from RAM.
    FetchFromDevice,
}

impl fmt::Display for BusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BusError::Unmapped => write!(f, "nothing is mapped there"),
            BusError::Unaligned => write!(f, "unaligned access (alignment fault on hardware)"),
            BusError::DeviceRegister { device, offset } => {
                write!(f, "{device} register at offset {offset:#x} not implemented for this access")
            }
            BusError::FetchFromDevice => write!(f, "instruction fetch from a device"),
        }
    }
}

/// Sparse RAM: pages that have never been written read as zero.
#[derive(Default)]
struct Ram {
    pages: HashMap<u64, Box<[u8]>>,
}

impl Ram {
    fn read_byte(&self, address: u64) -> u8 {
        let page = address / PAGE_SIZE;
        let offset = (address % PAGE_SIZE) as usize;
        self.pages.get(&page).map_or(0, |bytes| bytes[offset])
    }

    fn write_byte(&mut self, address: u64, value: u8) {
        let page = address / PAGE_SIZE;
        let offset = (address % PAGE_SIZE) as usize;
        self.pages.entry(page).or_insert_with(|| vec![0; PAGE_SIZE as usize].into_boxed_slice())
            [offset] = value;
    }
}

/// The machine's address space.
pub struct Bus {
    ram: Ram,
    uart: Pl011,
}

impl Bus {
    /// Creates a bus with empty RAM and the given UART.
    pub fn new(uart: Pl011) -> Self {
        Bus { ram: Ram::default(), uart }
    }

    /// Copies `bytes` into RAM at `address`, as the firmware does with the kernel image.
    pub fn load(&mut self, address: u64, bytes: &[u8]) -> Result<(), BusError> {
        if address.checked_add(bytes.len() as u64).is_none_or(|end| end > RAM_END) {
            return Err(BusError::Unmapped);
        }
        for (address, &byte) in (address..).zip(bytes) {
            self.ram.write_byte(address, byte);
        }
        Ok(())
    }

    /// Fetches the instruction word at `address`.
    pub fn fetch(&self, address: u64) -> Result<u32, BusError> {
        if !address.is_multiple_of(4) {
            return Err(BusError::Unaligned);
        }
        if address >= RAM_END {
            return Err(BusError::FetchFromDevice);
        }
        Ok(self.read_ram(address, 4) as u32)
    }

    /// Reads `size` bytes (1, 2, 4 or 8) at `address`, little-endian.
    pub fn read(&mut self, address: u64, size: u64) -> Result<u64, BusError> {
        if !address.is_multiple_of(size) {
            return Err(BusError::Unaligned);
        }
        if address < RAM_END {
            return Ok(self.read_ram(address, size));
        }
        match device_at(address) {
            Some(("PL011 UART", offset)) if size == 4 => self
                .uart
                .read(offset)
                .map(u64::from)
                .ok_or(BusError::DeviceRegister { device: "PL011 UART", offset }),
            Some((device, offset)) => Err(BusError::DeviceRegister { device, offset }),
            None => Err(BusError::Unmapped),
        }
    }

    /// Writes the low `size` bytes (1, 2, 4 or 8) of `value` at `address`, little-endian.
    pub fn write(&mut self, address: u64, size: u64, value: u64) -> Result<(), BusError> {
        if !address.is_multiple_of(size) {
            return Err(BusError::Unaligned);
        }
        if address < RAM_END {
            for (index, address) in (address..address + size).enumerate() {
                self.ram.write_byte(address, (value >> (8 * index)) as u8);
            }
            return Ok(());
        }
        match device_at(address) {
            Some(("PL011 UART", offset)) if size == 4 => self
                .uart
                .write(offset, value as u32)
                .ok_or(BusError::DeviceRegister { device: "PL011 UART", offset }),
            Some((device, offset)) => Err(BusError::DeviceRegister { device, offset }),
            None => Err(BusError::Unmapped),
        }
    }

    fn read_ram(&self, address: u64, size: u64) -> u64 {
        (0..size).fold(0, |value, index| {
            value | u64::from(self.ram.read_byte(address + index)) << (8 * index)
        })
    }
}

/// Finds the device whose registers include `address`, and the offset within it.
fn device_at(address: u64) -> Option<(&'static str, u64)> {
    if (UART0_BASE..UART0_BASE + DEVICE_WINDOW).contains(&address) {
        return Some(("PL011 UART", address - UART0_BASE));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bus() -> Bus {
        Bus::new(Pl011::new(Box::new(std::io::sink())))
    }

    #[test]
    fn ram_starts_zeroed_and_keeps_writes() {
        let mut bus = bus();
        assert_eq!(bus.read(0x1000, 8), Ok(0));
        bus.write(0x1000, 4, 0xdead_beef).unwrap();
        assert_eq!(bus.read(0x1000, 8), Ok(0xdead_beef));
        assert_eq!(bus.read(0x1002, 2), Ok(0xdead));
    }

    #[test]
    fn rejects_unaligned_and_unmapped_accesses() {
        let mut bus = bus();
        assert_eq!(bus.read(0x1004, 8), Err(BusError::Unaligned));
        assert_eq!(bus.read(0x3F00_0000, 4), Err(BusError::Unmapped));
        assert_eq!(bus.fetch(0x3F20_1000), Err(BusError::FetchFromDevice));
    }

    #[test]
    fn rejects_unimplemented_uart_registers_and_widths() {
        let mut bus = bus();
        assert_eq!(bus.read(UART0_BASE + 0x18, 4), Ok(0x90));
        let error = BusError::DeviceRegister { device: "PL011 UART", offset: 0x18 };
        assert_eq!(bus.read(UART0_BASE + 0x18, 8), Err(error));
        let error = BusError::DeviceRegister { device: "PL011 UART", offset: 0x48 };
        assert_eq!(bus.write(UART0_BASE + 0x48, 4, 0), Err(error));
    }
}
