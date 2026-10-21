//! The physical address space: RAM and memory-mapped devices.
//!
//! Every address Nebu does not model is an error, never silently ignored (ADR 0001).

use std::collections::HashMap;
use std::fmt;

use crate::gpio::Gpio;
use crate::local::{Controller, Local};
use crate::mailbox::{self, Mailbox, Request};
use crate::pl011::Pl011;
use crate::system_timer::SystemTimer;

/// RAM occupies the bottom of the address space, up to the peripherals.
const RAM_END: u64 = 0x3F00_0000;
/// The BCM2835 system timer, and the size of its register window.
const SYSTEM_TIMER_BASE: u64 = 0x3F00_3000;
const SYSTEM_TIMER_WINDOW: u64 = 0x1C;
/// The BCM2835 interrupt controller, and the size of its register window.
const INTERRUPT_CONTROLLER_BASE: u64 = 0x3F00_B200;
const INTERRUPT_CONTROLLER_WINDOW: u64 = 0x28;
/// The BCM2836 local peripherals, and the size of their register window.
const LOCAL_BASE: u64 = 0x4000_0000;
const LOCAL_WINDOW: u64 = 0x100;
/// The VideoCore mailboxes.
const MAILBOX_BASE: u64 = 0x3F00_B880;
/// The size of the mailboxes' register window: mailbox 0, then mailbox 1.
const MAILBOX_WINDOW: u64 = 0x40;
/// The GPIO controller.
const GPIO_BASE: u64 = 0x3F20_0000;
/// The PL011 UART (UART0).
const UART0_BASE: u64 = 0x3F20_1000;
/// The size of the other devices' register windows.
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
    /// A use of the firmware's mailbox that Nebu does not model.
    Mailbox(&'static str),
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
            BusError::Mailbox(what) => write!(f, "mailbox: not implemented: {what}"),
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
    gpio: Gpio,
    uart: Pl011,
    mailbox: Mailbox,
    local: Local,
    controller: Controller,
    system_timer: SystemTimer,
}

impl Bus {
    /// Creates a bus with empty RAM, GPIO in its reset state and the given UART.
    pub fn new(uart: Pl011) -> Self {
        Bus {
            ram: Ram::default(),
            gpio: Gpio::default(),
            uart,
            mailbox: Mailbox::default(),
            local: Local::default(),
            controller: Controller::default(),
            system_timer: SystemTimer::default(),
        }
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
            Some((device @ "GPIO", offset)) if size == 4 => self
                .gpio
                .read(offset)
                .map(u64::from)
                .ok_or(BusError::DeviceRegister { device, offset }),
            Some((device @ "PL011 UART", offset)) if size == 4 => self
                .uart
                .read(offset)
                .map(u64::from)
                .ok_or(BusError::DeviceRegister { device, offset }),
            Some((device @ "local peripherals", offset)) if size == 4 => self
                .local
                .read(offset)
                .map(u64::from)
                .ok_or(BusError::DeviceRegister { device, offset }),
            Some((device @ "interrupt controller", offset)) if size == 4 => self
                .controller
                .read(offset, self.device_levels())
                .map(u64::from)
                .ok_or(BusError::DeviceRegister { device, offset }),
            Some((device @ "system timer", offset)) if size == 4 => self
                .system_timer
                .read(offset)
                .map(u64::from)
                .ok_or(BusError::DeviceRegister { device, offset }),
            Some((device @ "mailbox", offset)) if size == 4 => self
                .mailbox
                .read(offset)
                .map_err(BusError::Mailbox)?
                .map(u64::from)
                .ok_or(BusError::DeviceRegister { device, offset }),
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
            Some((device @ "GPIO", offset)) if size == 4 => self
                .gpio
                .write(offset, value as u32)
                .ok_or(BusError::DeviceRegister { device, offset }),
            Some((device @ "PL011 UART", offset)) if size == 4 => self
                .uart
                .write(offset, value as u32)
                .ok_or(BusError::DeviceRegister { device, offset }),
            Some((device @ "local peripherals", offset)) if size == 4 => self
                .local
                .write(offset, value as u32)
                .ok_or(BusError::DeviceRegister { device, offset }),
            Some((device @ "interrupt controller", offset)) if size == 4 => {
                self.controller
                    .write(offset, value as u32)
                    .ok_or(BusError::DeviceRegister { device, offset })?;
                self.update_gpu_line();
                Ok(())
            }
            Some((device @ "system timer", offset)) if size == 4 => {
                self.system_timer
                    .write(offset, value as u32)
                    .ok_or(BusError::DeviceRegister { device, offset })?;
                self.update_gpu_line();
                Ok(())
            }
            Some((device @ "mailbox", offset)) if size == 4 => {
                match self.mailbox.write(offset, value as u32).map_err(BusError::Mailbox)? {
                    Some(Request::Property { address }) => self.answer_property(address),
                    None => Err(BusError::DeviceRegister { device, offset }),
                }
            }
            Some((device, offset)) => Err(BusError::DeviceRegister { device, offset }),
            None => Err(BusError::Unmapped),
        }
    }

    /// Records whether core 0's virtual timer is raising its interrupt (ADR 0016).
    pub fn set_virtual_timer(&mut self, raised: bool) {
        self.local.set_virtual_timer(raised);
    }

    /// Moves the devices on to the time when the core's counter reads `ticks`, and
    /// raises or lowers the BCM2835 line to match (ADR 0017).
    pub fn advance(&mut self, ticks: u64) {
        if self.system_timer.advance(ticks) {
            self.update_gpu_line();
        }
    }

    /// Raises the BCM2835 line while an enabled interrupt is raised. The levels and
    /// the enables change only when the system timer counts or either device is
    /// written.
    fn update_gpu_line(&mut self) {
        self.local.set_gpu(self.device_levels() & self.controller.enabled() != 0);
    }

    /// The device interrupts raised: only the system timer's.
    fn device_levels(&self) -> u64 {
        u64::from(self.system_timer.levels())
    }

    /// Whether core 0 has an IRQ pending.
    pub fn irq_pending(&self) -> bool {
        self.local.irq_pending()
    }

    /// Whether core 0's virtual timer raises an IRQ once it fires.
    pub fn virtual_timer_routed(&self) -> bool {
        self.local.virtual_timer_routed()
    }

    /// The core's counter at which the next enabled device interrupt is raised, if
    /// one ever will be.
    pub fn next_device_interrupt(&self) -> Option<u64> {
        self.system_timer.next_match(self.controller.enabled() as u32)
    }

    /// Answers the property message at `address` in RAM, as the firmware would.
    fn answer_property(&mut self, address: u64) -> Result<(), BusError> {
        let size = self.read_ram(address, 4);
        // The firmware's own limit is far larger, but NeOS's messages are small.
        if !(12..=0x1000).contains(&size) || !size.is_multiple_of(4) || address + size > RAM_END {
            return Err(BusError::Mailbox("a property message of that size or place"));
        }
        let mut message: Vec<u32> = (address..address + size)
            .step_by(4)
            .map(|word| self.read_ram(word, 4) as u32)
            .collect();
        mailbox::answer(&mut message).map_err(BusError::Mailbox)?;
        for (word, value) in (address..).step_by(4).zip(message) {
            for index in 0..4 {
                self.ram.write_byte(word + index, (value >> (8 * index)) as u8);
            }
        }
        Ok(())
    }

    fn read_ram(&self, address: u64, size: u64) -> u64 {
        (0..size).fold(0, |value, index| {
            value | u64::from(self.ram.read_byte(address + index)) << (8 * index)
        })
    }
}

/// Finds the device whose registers include `address`, and the offset within it.
fn device_at(address: u64) -> Option<(&'static str, u64)> {
    if (SYSTEM_TIMER_BASE..SYSTEM_TIMER_BASE + SYSTEM_TIMER_WINDOW).contains(&address) {
        return Some(("system timer", address - SYSTEM_TIMER_BASE));
    }
    let interrupt_controller =
        INTERRUPT_CONTROLLER_BASE..INTERRUPT_CONTROLLER_BASE + INTERRUPT_CONTROLLER_WINDOW;
    if interrupt_controller.contains(&address) {
        return Some(("interrupt controller", address - INTERRUPT_CONTROLLER_BASE));
    }
    if (LOCAL_BASE..LOCAL_BASE + LOCAL_WINDOW).contains(&address) {
        return Some(("local peripherals", address - LOCAL_BASE));
    }
    if (MAILBOX_BASE..MAILBOX_BASE + MAILBOX_WINDOW).contains(&address) {
        return Some(("mailbox", address - MAILBOX_BASE));
    }
    if (GPIO_BASE..GPIO_BASE + DEVICE_WINDOW).contains(&address) {
        return Some(("GPIO", address - GPIO_BASE));
    }
    if (UART0_BASE..UART0_BASE + DEVICE_WINDOW).contains(&address) {
        return Some(("PL011 UART", address - UART0_BASE));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bus() -> Bus {
        Bus::new(Pl011::new(Box::new(std::io::sink()), std::sync::mpsc::channel().1))
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

    #[test]
    fn answers_property_messages_in_ram() {
        let mut bus = bus();
        for (address, word) in (0x1000..).step_by(4).zip([32, 0, 0x0001_0005, 8, 0, 0, 0, 0]) {
            bus.write(address, 4, word).unwrap();
        }
        assert_eq!(bus.read(MAILBOX_BASE + 0x18, 4), Ok(1 << 30));
        bus.write(MAILBOX_BASE + 0x20, 4, 0xC000_1008).unwrap();
        assert_eq!(bus.read(MAILBOX_BASE + 0x18, 4), Ok(0));
        assert_eq!(bus.read(MAILBOX_BASE, 4), Ok(0xC000_1008));
        assert_eq!(bus.read(0x1004, 4), Ok(0x8000_0000));
        assert_eq!(bus.read(0x1018, 4), Ok(0x3C00_0000));
    }
}
