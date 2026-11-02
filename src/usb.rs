//! The DesignWare "DWC2" USB host controller, in buffer DMA mode, as far as NeOS
//! uses it, with the devices behind its root port (ADR 0024).
//!
//! Nebu follows QEMU 8.2's model (`hw/usb/hcd-dwc2.c`): its registers keep the same
//! values and change in the same ways, and a channel runs its whole transfer the
//! moment it is enabled, a packet at a time, moving its data straight to or from
//! RAM. A start-of-frame interrupt is raised every millisecond while a device is
//! attached. Unlike QEMU, Nebu halts on what NeOS does not use, and that QEMU
//! would ignore or get wrong: slave mode, split transactions, low-speed devices,
//! bulk and isochronous transfers, and a transfer no device answers.

use crate::usb_devices::{Device, Key, Reply, Token};

/// RAM, which the controller reads and writes for DMA.
pub trait Memory {
    fn read_byte(&self, address: u64) -> u8;
    fn write_byte(&mut self, address: u64, value: u8);
}

/// The registers NeOS uses, as offsets from the controller's base at 0x3F98_0000.
const GAHBCFG: u64 = 0x008;
const GUSBCFG: u64 = 0x00C;
const GRSTCTL: u64 = 0x010;
const GINTSTS: u64 = 0x014;
const GINTMSK: u64 = 0x018;
const GRXFSIZ: u64 = 0x024;
const GNPTXFSIZ: u64 = 0x028;
const GSNPSID: u64 = 0x040;
const HPTXFSIZ: u64 = 0x100;
const HCFG: u64 = 0x400;
const HFNUM: u64 = 0x408;
const HAINT: u64 = 0x414;
const HAINTMSK: u64 = 0x418;
const HPRT0: u64 = 0x440;
const PCGCTL: u64 = 0xE00;
/// The channels' registers, from 0x500, 0x20 bytes a channel.
const CHANNEL_BASE: u64 = 0x500;
const CHANNEL_STRIDE: u64 = 0x20;
const HCCHAR: u64 = 0x00;
const HCSPLT: u64 = 0x04;
const HCINT: u64 = 0x08;
const HCINTMSK: u64 = 0x0C;
const HCTSIZ: u64 = 0x10;
const HCDMA: u64 = 0x14;
/// How many channels QEMU's controller has, as the Pi's does.
const CHANNELS: usize = 8;

/// What QEMU's GSNPSID says: "OT2", version 2.94a.
const SYNOPSYS_ID: u32 = 0x4F54_294A;
/// GAHBCFG: interrupts on, the BCM2835's "wait for AXI writes", and DMA.
const GLOBAL_INTERRUPTS: u32 = 1 << 0;
const WAIT_AXI_WRITES: u32 = 1 << 4;
const DMA_ENABLE: u32 = 1 << 5;
/// GRSTCTL: core soft reset, the FIFO flushes and which FIFO, and the bus idle.
const CORE_RESET: u32 = 1 << 0;
const RX_FLUSH: u32 = 1 << 4;
const TX_FLUSH: u32 = 1 << 5;
const TX_FIFO_NUMBER: u32 = 0x1F << 6;
const AHB_IDLE: u32 = 1 << 31;
const SELF_CLEARING: u32 = CORE_RESET | RX_FLUSH | TX_FLUSH;
/// GINTSTS and GINTMSK: host mode, the FIFOs empty, start of frame, the port, the
/// channels, and the connector ID changing, which QEMU sets at reset.
const HOST_MODE: u32 = 1 << 0;
const NON_PERIODIC_EMPTY: u32 = 1 << 5;
const START_OF_FRAME: u32 = 1 << 3;
const PORT_INTERRUPT: u32 = 1 << 24;
const CHANNEL_INTERRUPT: u32 = 1 << 25;
const PERIODIC_EMPTY: u32 = 1 << 26;
const CONNECTOR_ID_CHANGED: u32 = 1 << 28;
const READ_ONLY_STATUS: u32 =
    HOST_MODE | NON_PERIODIC_EMPTY | PORT_INTERRUPT | CHANNEL_INTERRUPT | PERIODIC_EMPTY;
const MODELLED_MASK: u32 = START_OF_FRAME | PORT_INTERRUPT | CHANNEL_INTERRUPT;
/// HPRT0: connected, connection detected, enabled, enabling changed, over-current
/// changed, resume, suspend, reset, power, and the speed (1, full).
const CONNECTED: u32 = 1 << 0;
const CONNECTION_DETECTED: u32 = 1 << 1;
const ENABLED: u32 = 1 << 2;
const ENABLE_CHANGED: u32 = 1 << 3;
const OVERCURRENT_CHANGED: u32 = 1 << 5;
const RESUME: u32 = 1 << 6;
const SUSPEND: u32 = 1 << 7;
const PORT_RESET: u32 = 1 << 8;
const PORT_POWER: u32 = 1 << 12;
const FULL_SPEED: u32 = 1 << 17;
const PORT_READ_ONLY: u32 = 0b11 << 17 | 0b11 << 10 | 1 << 4 | CONNECTED;
const PORT_WRITE_TO_CLEAR: u32 =
    CONNECTION_DETECTED | ENABLED | ENABLE_CHANGED | OVERCURRENT_CHANGED;
const PORT_UNMODELLED: u32 = RESUME | SUSPEND | 0xF << 13;
/// HCCHAR's fields.
const MAX_PACKET_MASK: u32 = 0x7FF;
const ENDPOINT_SHIFT: u32 = 11;
const DIRECTION_IN: u32 = 1 << 15;
const LOW_SPEED_DEVICE: u32 = 1 << 17;
const TYPE_SHIFT: u32 = 18;
const ADDRESS_SHIFT: u32 = 22;
const CHANNEL_DISABLE: u32 = 1 << 30;
const CHANNEL_ENABLE: u32 = 1 << 31;
const CONTROL: u32 = 0;
const INTERRUPT: u32 = 3;
/// HCINT's bits, and those that exist.
const TRANSFER_COMPLETE: u32 = 1 << 0;
const HALTED: u32 = 1 << 1;
const STALL: u32 = 1 << 3;
const NAK: u32 = 1 << 4;
const CHANNEL_BITS: u32 = 0x3FFF;
/// HCTSIZ's fields.
const SIZE_MASK: u32 = (1 << 19) - 1;
const PACKETS_SHIFT: u32 = 19;
const PACKETS_MASK: u32 = 0x3FF;
const PID_SHIFT: u32 = 29;
const PID_SETUP: u32 = 3;
const PID_DATA2: u32 = 1;

/// Where the GPU's bus sees RAM, uncached, which is what NeOS gives DMA masters.
const BUS_RAM: u64 = 0xC000_0000;
/// The core's counts in a 1 ms frame, at 62.5 MHz (ADR 0016), and the frame
/// interval in full-speed bit times, less one, which HFNUM counts down.
const COUNTS_PER_FRAME: u64 = 62_500;
const FRAME_INTERVAL: u64 = 12_000 - 1;

/// A channel's registers.
#[derive(Clone, Copy, Default)]
struct Channel {
    characteristics: u32,
    split: u32,
    interrupts: u32,
    mask: u32,
    size: u32,
    dma: u32,
}

/// The controller's state.
pub struct Dwc2 {
    ahb: u32,
    usb: u32,
    reset: u32,
    status: u32,
    mask: u32,
    rx_fifo: u32,
    non_periodic_fifo: u32,
    periodic_fifo: u32,
    host: u32,
    channel_interrupts: u32,
    channel_mask: u32,
    port: u32,
    channels: [Channel; CHANNELS],
    /// The device on the root port, if any: QEMU's hub, with a keyboard.
    root: Option<Device>,
    /// The core's counter, and the frames since the device was attached at boot.
    now: u64,
    frames: u64,
}

impl Dwc2 {
    /// A controller as QEMU resets it, with a hub and keyboard attached if
    /// `keyboard` holds the keys to type on it.
    pub fn new(keyboard: Option<Vec<Key>>) -> Dwc2 {
        let mut dwc2 = Dwc2 {
            ahb: 0,
            usb: 5 << 10,
            reset: AHB_IDLE,
            status: CONNECTOR_ID_CHANGED | PERIODIC_EMPTY | NON_PERIODIC_EMPTY | HOST_MODE,
            mask: 0,
            rx_fifo: 1024,
            non_periodic_fifo: 1024 << 16,
            periodic_fifo: 500 << 16,
            host: 2 << 8,
            channel_interrupts: 0,
            channel_mask: 0,
            port: PORT_POWER,
            channels: [Channel::default(); CHANNELS],
            root: None,
            now: 0,
            frames: 0,
        };
        if let Some(keys) = keyboard {
            dwc2.root = Some(Device::hub_with_keyboard(keys));
            dwc2.port |= FULL_SPEED | CONNECTION_DETECTED | CONNECTED;
            dwc2.status |= PORT_INTERRUPT;
        }
        dwc2
    }

    /// Reads the register at `offset`, if Nebu implements it.
    pub fn read(&mut self, offset: u64) -> Option<u32> {
        if let Some((number, register)) = channel_register(offset) {
            let channel = &self.channels[number];
            return match register {
                HCCHAR => Some(channel.characteristics),
                HCSPLT => Some(channel.split),
                HCINT => Some(channel.interrupts),
                HCINTMSK => Some(channel.mask),
                HCTSIZ => Some(channel.size),
                HCDMA => Some(channel.dma),
                _ => None,
            };
        }
        Some(match offset {
            GAHBCFG => self.ahb,
            GUSBCFG => self.usb,
            GRSTCTL => {
                // Reading clears the bits that clear themselves: the resets are
                // done at once.
                self.reset &= !SELF_CLEARING;
                self.reset
            }
            GINTSTS => self.status,
            GINTMSK => self.mask,
            GRXFSIZ => self.rx_fifo,
            GNPTXFSIZ => self.non_periodic_fifo,
            GSNPSID => SYNOPSYS_ID,
            HPTXFSIZ => self.periodic_fifo,
            HCFG => self.host,
            HFNUM => self.frame_number(),
            HAINT => self.channel_interrupts,
            HAINTMSK => self.channel_mask,
            HPRT0 => self.port,
            PCGCTL => 0,
            _ => return None,
        })
    }

    /// Writes the register at `offset`, if Nebu implements it and models the value.
    /// Enabling a channel runs its transfer, through `memory`.
    pub fn write(
        &mut self,
        offset: u64,
        value: u32,
        memory: &mut dyn Memory,
    ) -> Result<Option<()>, &'static str> {
        if let Some((number, register)) = channel_register(offset) {
            return self.write_channel(number, register, value, memory);
        }
        match offset {
            GAHBCFG if value & !(GLOBAL_INTERRUPTS | WAIT_AXI_WRITES | DMA_ENABLE) == 0 => {
                self.ahb = value;
            }
            GUSBCFG => self.usb = value,
            // A core soft reset changes nothing, as in QEMU.
            GRSTCTL if value & !(SELF_CLEARING | TX_FIFO_NUMBER) == 0 => {
                self.reset = value | AHB_IDLE | self.reset & SELF_CLEARING;
            }
            GINTSTS => {
                let read_only = self.status & (READ_ONLY_STATUS);
                self.status = self.status & !value | read_only;
            }
            GINTMSK if value & !MODELLED_MASK == 0 => self.mask = value,
            GRXFSIZ => self.rx_fifo = value,
            GNPTXFSIZ => self.non_periodic_fifo = value,
            HPTXFSIZ => self.periodic_fifo = value,
            HCFG => self.host = value,
            HAINTMSK => {
                self.channel_mask = value & 0xFFFF;
                self.update_channel_interrupt();
            }
            HPRT0 if value & PORT_UNMODELLED == 0 => self.write_port(value),
            PCGCTL if value == 0 => {}
            _ => return Ok(None),
        }
        Ok(Some(()))
    }

    /// Writes HPRT0, as QEMU does: the read-only bits stay, the change bits and
    /// "enabled" clear when 1 is written, and the end of a reset enables the port
    /// and resets the device.
    fn write_port(&mut self, value: u32) {
        let old = self.port;
        let mut value = value & !PORT_READ_ONLY | old & PORT_READ_ONLY;
        if old & ENABLED == 0 {
            value &= !ENABLED;
        }
        let cleared = old & PORT_WRITE_TO_CLEAR & !value;
        value = value & !PORT_WRITE_TO_CLEAR | cleared;
        let mut reset = false;
        if value & PORT_RESET == 0 && old & PORT_RESET != 0 && self.root.is_some() {
            value |= ENABLED | ENABLE_CHANGED;
            reset = true;
        }
        if value & (OVERCURRENT_CHANGED | ENABLE_CHANGED | CONNECTION_DETECTED) != 0 {
            self.status |= PORT_INTERRUPT;
        } else {
            self.status &= !PORT_INTERRUPT;
        }
        if reset {
            if let Some(device) = &mut self.root {
                device.reset();
            }
            value &= !CONNECTION_DETECTED;
        }
        self.port = value;
    }

    /// Writes one of channel `number`'s registers.
    fn write_channel(
        &mut self,
        number: usize,
        register: u64,
        value: u32,
        memory: &mut dyn Memory,
    ) -> Result<Option<()>, &'static str> {
        let channel = &mut self.channels[number];
        match register {
            HCCHAR => {
                let old = channel.characteristics;
                if value & CHANNEL_DISABLE != 0 && old & CHANNEL_DISABLE == 0 {
                    // Disabling halts it at once, since no transfer is ever left
                    // running.
                    channel.characteristics = value & !(CHANNEL_ENABLE | CHANNEL_DISABLE);
                    channel.interrupts |= HALTED;
                } else if value & CHANNEL_ENABLE != 0 && old & CHANNEL_ENABLE == 0 {
                    channel.characteristics = value & !CHANNEL_DISABLE;
                    self.transfer(number, memory)?;
                } else {
                    channel.characteristics = value | old & (CHANNEL_DISABLE | CHANNEL_ENABLE);
                }
            }
            HCSPLT => channel.split = value,
            HCINT => channel.interrupts &= !value & CHANNEL_BITS,
            HCINTMSK => channel.mask = value & CHANNEL_BITS,
            HCTSIZ => channel.size = value,
            HCDMA => channel.dma = value,
            _ => return Ok(None),
        }
        self.update_channel_interrupt();
        Ok(Some(()))
    }

    /// Runs channel `number`'s transfer, a packet at a time, as QEMU does, and
    /// halts the channel.
    fn transfer(&mut self, number: usize, memory: &mut dyn Memory) -> Result<(), &'static str> {
        if self.ahb & DMA_ENABLE == 0 {
            return Err("USB: a transfer in slave mode, which Nebu does not model");
        }
        let channel = self.channels[number];
        let characteristics = channel.characteristics;
        if channel.split != 0 {
            return Err("USB: a split transaction, which Nebu does not model");
        }
        if characteristics & LOW_SPEED_DEVICE != 0 {
            return Err("USB: a transfer to a low-speed device, which Nebu does not model");
        }
        let max_packet = (characteristics & MAX_PACKET_MASK) as usize;
        if max_packet == 0 {
            return Err("USB: a transfer with a largest packet of 0");
        }
        let kind = characteristics >> TYPE_SHIFT & 3;
        if kind != CONTROL && kind != INTERRUPT {
            return Err("USB: a bulk or isochronous transfer, which Nebu does not model");
        }
        let pid = channel.size >> PID_SHIFT & 3;
        if pid == PID_DATA2 {
            return Err("USB: a transfer with PID DATA2, which only isochronous ones use");
        }
        let token = match (kind, pid, characteristics & DIRECTION_IN != 0) {
            (CONTROL, PID_SETUP, _) => Token::Setup,
            (_, _, true) => Token::In,
            (_, _, false) => Token::Out,
        };
        let endpoint = (characteristics >> ENDPOINT_SHIFT & 0xF) as u8;
        let address = (characteristics >> ADDRESS_SHIFT & 0x7F) as u8;
        let enabled = self.port & ENABLED != 0;
        let device = self
            .root
            .as_mut()
            .filter(|_| enabled)
            .and_then(|root| root.find(address))
            .ok_or("USB: a transfer to an address no device answers")?;

        let mut length = (channel.size & SIZE_MASK) as usize;
        let mut packets = channel.size >> PACKETS_SHIFT & PACKETS_MASK;
        let mut dma = channel.dma;
        let interrupts = loop {
            let part = length.min(max_packet);
            let data = match token {
                Token::In => Vec::new(),
                _ => read_dma(memory, dma, part)?,
            };
            match device.handle(endpoint, token, &data, part)? {
                Reply::Ack { actual, data } => {
                    if token == Token::In {
                        write_dma(memory, dma, &data)?;
                    }
                    let short = actual % max_packet != 0;
                    let sent = actual.div_ceil(max_packet) as u32;
                    packets -= sent.min(packets);
                    length -= actual.min(length);
                    dma = dma.wrapping_add(actual as u32);
                    if packets == 0 || length == 0 || actual == 0 || (short && token == Token::In) {
                        break TRANSFER_COMPLETE | HALTED;
                    }
                }
                Reply::Nak if kind == CONTROL => {
                    return Err(
                        "USB: a NAK in a control transfer, which Nebu's devices never send",
                    );
                }
                Reply::Nak => break NAK | HALTED,
                Reply::Stall => break STALL | HALTED,
            }
        };
        let channel = &mut self.channels[number];
        channel.size = channel.size & !(SIZE_MASK | PACKETS_MASK << PACKETS_SHIFT)
            | length as u32 & SIZE_MASK
            | (packets & PACKETS_MASK) << PACKETS_SHIFT;
        channel.dma = dma;
        channel.characteristics &= !CHANNEL_ENABLE;
        channel.interrupts |= interrupts;
        Ok(())
    }

    /// Recomputes HAINT from the channels, and GINTSTS's channel bit from it.
    fn update_channel_interrupt(&mut self) {
        self.channel_interrupts = self
            .channels
            .iter()
            .enumerate()
            .filter(|(_, channel)| channel.interrupts & channel.mask != 0)
            .fold(0, |bits, (number, _)| bits | 1 << number);
        if self.channel_interrupts & self.channel_mask != 0 {
            self.status |= CHANNEL_INTERRUPT;
        } else {
            self.status &= !CHANNEL_INTERRUPT;
        }
    }

    /// HFNUM: the frame number, which QEMU starts at 0x3FFF, and the bit times
    /// left in the frame.
    fn frame_number(&self) -> u32 {
        if self.root.is_none() {
            return 0x3FFF;
        }
        let into_frame = (self.now % COUNTS_PER_FRAME) * (FRAME_INTERVAL + 1) / COUNTS_PER_FRAME;
        let remaining = FRAME_INTERVAL.saturating_sub(into_frame) as u32;
        remaining << 16 | (self.frames + 0x3FFF) as u32 & 0x3FFF
    }

    /// Moves on to when the core's counter reads `ticks`, raising the start-of-frame
    /// interrupt at each new frame while a device is attached. Returns whether the
    /// interrupt's level may have changed.
    pub fn advance(&mut self, ticks: u64) -> bool {
        self.now = ticks;
        if self.root.is_none() || ticks < (self.frames + 1) * COUNTS_PER_FRAME {
            return false;
        }
        self.frames = ticks / COUNTS_PER_FRAME;
        self.status |= START_OF_FRAME;
        true
    }

    /// Whether the controller is raising its interrupt.
    pub fn interrupt(&self) -> bool {
        self.ahb & GLOBAL_INTERRUPTS != 0 && self.status & self.mask != 0
    }

    /// The core's counter at which the controller next raises its interrupt by
    /// itself, at the next frame, if the start of frame's is enabled.
    pub fn next_interrupt(&self) -> Option<u64> {
        let enabled = self.ahb & GLOBAL_INTERRUPTS != 0 && self.mask & START_OF_FRAME != 0;
        (self.root.is_some() && enabled).then_some((self.frames + 1) * COUNTS_PER_FRAME)
    }
}

/// The channel and the register within it at `offset`, if it is a channel's.
fn channel_register(offset: u64) -> Option<(usize, u64)> {
    let relative = offset.checked_sub(CHANNEL_BASE)?;
    let number = (relative / CHANNEL_STRIDE) as usize;
    (number < CHANNELS).then_some((number, relative % CHANNEL_STRIDE))
}

/// The RAM address of `length` bytes at bus address `address`.
fn ram_address(address: u32, length: usize) -> Result<u64, &'static str> {
    let address = u64::from(address)
        .checked_sub(BUS_RAM)
        .ok_or("USB: DMA through a bus address other than RAM's uncached alias")?;
    if address + length as u64 > crate::bus::RAM_END {
        return Err("USB: DMA past the end of RAM");
    }
    Ok(address)
}

fn read_dma(memory: &dyn Memory, address: u32, length: usize) -> Result<Vec<u8>, &'static str> {
    let start = ram_address(address, length)?;
    Ok((start..start + length as u64).map(|address| memory.read_byte(address)).collect())
}

fn write_dma(memory: &mut dyn Memory, address: u32, data: &[u8]) -> Result<(), &'static str> {
    let start = ram_address(address, data.len())?;
    for (address, &byte) in (start..).zip(data) {
        memory.write_byte(address, byte);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Ram(HashMap<u64, u8>);

    impl Memory for Ram {
        fn read_byte(&self, address: u64) -> u8 {
            self.0.get(&address).copied().unwrap_or(0)
        }
        fn write_byte(&mut self, address: u64, value: u8) {
            self.0.insert(address, value);
        }
    }

    const BUFFER: u32 = 0xC010_0000;

    fn write(dwc2: &mut Dwc2, ram: &mut Ram, offset: u64, value: u32) {
        assert_eq!(dwc2.write(offset, value, ram), Ok(Some(())), "writing {offset:#x}");
    }

    /// Runs one stage of a transfer on channel 0 to address 0's endpoint 0, and
    /// returns HCINT.
    fn stage(dwc2: &mut Dwc2, ram: &mut Ram, pid: u32, is_in: bool, length: u32) -> u32 {
        let packets = length.div_ceil(8).max(1);
        write(dwc2, ram, CHANNEL_BASE + HCINT, CHANNEL_BITS);
        write(
            dwc2,
            ram,
            CHANNEL_BASE + HCTSIZ,
            length | packets << PACKETS_SHIFT | pid << PID_SHIFT,
        );
        write(dwc2, ram, CHANNEL_BASE + HCDMA, BUFFER);
        let direction = if is_in { DIRECTION_IN } else { 0 };
        write(dwc2, ram, CHANNEL_BASE + HCCHAR, 8 | direction | CHANNEL_ENABLE);
        dwc2.read(CHANNEL_BASE + HCINT).unwrap()
    }

    #[test]
    fn starts_as_qemu_does_with_a_device_attached() {
        let mut dwc2 = Dwc2::new(Some(Vec::new()));
        assert_eq!(dwc2.read(GSNPSID), Some(SYNOPSYS_ID));
        assert_eq!(
            dwc2.read(HPRT0),
            Some(PORT_POWER | FULL_SPEED | CONNECTION_DETECTED | CONNECTED)
        );
        assert_ne!(dwc2.read(GINTSTS).unwrap() & PORT_INTERRUPT, 0);
        assert_eq!(Dwc2::new(None).read(HPRT0), Some(PORT_POWER));
    }

    #[test]
    fn reads_a_descriptor_after_a_port_reset() {
        let mut dwc2 = Dwc2::new(Some(Vec::new()));
        let mut ram = Ram::default();
        write(&mut dwc2, &mut ram, GAHBCFG, DMA_ENABLE);
        write(&mut dwc2, &mut ram, HPRT0, PORT_POWER | PORT_RESET);
        write(&mut dwc2, &mut ram, HPRT0, PORT_POWER);
        let port = dwc2.read(HPRT0).unwrap();
        assert_eq!(
            port & (ENABLED | ENABLE_CHANGED | CONNECTION_DETECTED),
            ENABLED | ENABLE_CHANGED
        );

        // GET_DESCRIPTOR (device), 18 bytes.
        for (offset, byte) in [0x80, 6, 0, 1, 0, 0, 18, 0].into_iter().enumerate() {
            ram.write_byte(0x10_0000 + offset as u64, byte);
        }
        assert_eq!(stage(&mut dwc2, &mut ram, PID_SETUP, false, 8), TRANSFER_COMPLETE | HALTED);
        assert_eq!(stage(&mut dwc2, &mut ram, 2, true, 24), TRANSFER_COMPLETE | HALTED);
        assert_eq!(dwc2.read(CHANNEL_BASE + HCTSIZ).unwrap() & SIZE_MASK, 6);
        assert_eq!([0x0A, 0x0B].map(|at| ram.read_byte(0x10_0000 + at)), [0xAA, 0x55]);
        assert_eq!(stage(&mut dwc2, &mut ram, 2, false, 0), TRANSFER_COMPLETE | HALTED);
    }

    #[test]
    fn halts_on_what_nebu_does_not_model() {
        let mut dwc2 = Dwc2::new(Some(Vec::new()));
        let mut ram = Ram::default();
        // Slave mode, then a transfer before the port is enabled.
        assert!(dwc2.write(CHANNEL_BASE + HCCHAR, 8 | CHANNEL_ENABLE, &mut ram).is_err());
        write(&mut dwc2, &mut ram, GAHBCFG, DMA_ENABLE);
        write(&mut dwc2, &mut ram, CHANNEL_BASE + HCCHAR, 8 | CHANNEL_DISABLE);
        assert!(dwc2.write(CHANNEL_BASE + HCCHAR, 8 | CHANNEL_ENABLE, &mut ram).is_err());
        assert_eq!(dwc2.write(GINTMSK, 1 << 7, &mut ram), Ok(None));
    }

    #[test]
    fn raises_start_of_frame_each_millisecond() {
        let mut dwc2 = Dwc2::new(Some(Vec::new()));
        let mut ram = Ram::default();
        write(&mut dwc2, &mut ram, GINTSTS, u32::MAX);
        write(&mut dwc2, &mut ram, GINTMSK, START_OF_FRAME);
        write(&mut dwc2, &mut ram, GAHBCFG, GLOBAL_INTERRUPTS);
        assert!(!dwc2.interrupt());
        assert_eq!(dwc2.next_interrupt(), Some(COUNTS_PER_FRAME));
        assert!(dwc2.advance(COUNTS_PER_FRAME));
        assert!(dwc2.interrupt());
        assert_eq!(dwc2.read(HFNUM).unwrap() & 0x3FFF, 0);
    }
}
