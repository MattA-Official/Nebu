//! The VideoCore mailbox, as far as NeOS uses it: asking the firmware how much memory
//! the CPU has (ADR 0011), and for a framebuffer (ADR 0023).
//!
//! The CPU writes the bus address of a message, with a channel number in its low 4
//! bits, to mailbox 1's WRITE register. The firmware answers in the message itself,
//! then puts the same word in mailbox 0 for the CPU to read. Nebu answers at once,
//! as QEMU does. Register offsets are from the BCM2835 mailbox documentation in the
//! Raspberry Pi firmware wiki.

use std::collections::VecDeque;

/// Mailbox 0's READ register: takes the oldest answer.
const READ: u64 = 0x00;
/// Mailbox 0's STATUS register.
const STATUS: u64 = 0x18;
/// Mailbox 1's WRITE register: sends a message to the firmware.
const WRITE: u64 = 0x20;
/// Mailbox 1's STATUS register, whose FULL bit says WRITE cannot take more yet.
const WRITE_STATUS: u64 = 0x38;
/// STATUS bit: there is nothing to read.
const EMPTY: u32 = 1 << 30;

/// The channel for the property interface: messages made of tags.
pub const PROPERTY_CHANNEL: u32 = 8;
/// Property tag: the base address and size of the CPU's memory.
const GET_ARM_MEMORY: u32 = 0x0001_0005;
/// Property tags for the framebuffer (ADR 0023).
const ALLOCATE_BUFFER: u32 = 0x0004_0001;
const GET_PHYSICAL_SIZE: u32 = 0x0004_0003;
const GET_PITCH: u32 = 0x0004_0008;
const SET_PHYSICAL_SIZE: u32 = 0x0004_8003;
const SET_VIRTUAL_SIZE: u32 = 0x0004_8004;
const SET_DEPTH: u32 = 0x0004_8005;
const SET_PIXEL_ORDER: u32 = 0x0004_8006;
const SET_VIRTUAL_OFFSET: u32 = 0x0004_8009;
/// Where QEMU's `raspi3b` puts the framebuffer: 1 MiB into the GPU's memory.
const FRAMEBUFFER_ADDRESS: u32 = 0x3C10_0000;
/// The GPU's memory ends where the peripherals start, so the buffer must fit below.
const GPU_MEMORY_END: u64 = 0x3F00_0000;
/// The response code for a message or tag the firmware has answered.
const RESPONSE_SUCCESS: u32 = 0x8000_0000;
/// The memory the firmware gives the CPU: QEMU's `raspi3b` gives 1 GiB minus the
/// GPU's 64 MiB, at address 0.
const ARM_MEMORY_SIZE: u32 = 0x3C00_0000;

/// What the CPU asked the mailbox to do, which only the bus, owning RAM, can finish.
pub enum Request {
    /// Answer the property message at this physical address.
    Property { address: u64 },
}

/// The framebuffer's settings, as the property tags set them. Its pixels are in
/// RAM, which the bus owns.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Framebuffer {
    /// The screen's size in pixels, which the virtual size must match.
    pub width: u32,
    pub height: u32,
    /// Bits per pixel: only 32 is supported.
    pub depth: u32,
    /// 1 for red, green, blue; 0 for blue, green, red.
    pub pixel_order: u32,
    /// Whether "allocate buffer" has been asked, so the screen shows the buffer.
    pub allocated: bool,
}

impl Default for Framebuffer {
    /// QEMU's default screen: 640×480, 16 bits per pixel, nothing allocated.
    fn default() -> Framebuffer {
        Framebuffer { width: 640, height: 480, depth: 16, pixel_order: 1, allocated: false }
    }
}

impl Framebuffer {
    /// The bytes from one line of pixels to the next.
    pub fn pitch(&self) -> u32 {
        self.width * self.depth / 8
    }

    /// The buffer's physical address and size in bytes.
    pub fn buffer(&self) -> (u64, u64) {
        (u64::from(FRAMEBUFFER_ADDRESS), u64::from(self.pitch()) * u64::from(self.height))
    }
}

/// The mailbox registers NeOS uses, and the firmware's framebuffer settings.
#[derive(Default)]
pub struct Mailbox {
    answers: VecDeque<u32>,
    framebuffer: Framebuffer,
}

impl Mailbox {
    /// Reads the 32-bit register at `offset`, or `None` if it is not implemented. The
    /// error says why, when there is more to say than that.
    pub fn read(&mut self, offset: u64) -> Result<Option<u32>, &'static str> {
        Ok(match offset {
            READ => Some(self.answers.pop_front().ok_or("reading an empty mailbox")?),
            // Nebu answers at once, so the mailbox is never full.
            STATUS => Some(if self.answers.is_empty() { EMPTY } else { 0 }),
            // Nebu answers each message as it is written, so mailbox 1 never fills.
            WRITE_STATUS => Some(0),
            _ => None,
        })
    }

    /// Writes the 32-bit register at `offset`. Returns `None` if it is not
    /// implemented, or the request for the bus to finish.
    pub fn write(&mut self, offset: u64, value: u32) -> Result<Option<Request>, &'static str> {
        if offset != WRITE {
            return Ok(None);
        }
        if value & 0xF != PROPERTY_CHANNEL {
            return Err("mailbox channels other than 8, the property interface");
        }
        // The top two bits of a bus address choose how the GPU caches it.
        let address = u64::from(value & 0x3FFF_FFF0);
        self.answers.push_back(value);
        Ok(Some(Request::Property { address }))
    }
}

impl Mailbox {
    /// The framebuffer, once it has been allocated.
    pub fn framebuffer(&self) -> Option<Framebuffer> {
        Some(self.framebuffer).filter(|framebuffer| framebuffer.allocated)
    }

    /// Answers a property message in place. `message` holds its words, starting with
    /// its size in bytes.
    pub fn answer(&mut self, message: &mut [u32]) -> Result<(), &'static str> {
        answer(message, &mut self.framebuffer)
    }
}

/// Answers a property message in place, changing `framebuffer` as its tags say.
fn answer(message: &mut [u32], framebuffer: &mut Framebuffer) -> Result<(), &'static str> {
    let mut index = 2;
    loop {
        let tag = *message.get(index).ok_or("a property message without an end tag")?;
        if tag == 0 {
            break;
        }
        let size = *message.get(index + 1).ok_or("a property tag cut short")? as usize;
        if !size.is_multiple_of(4) || index + 3 + size / 4 > message.len() {
            return Err("a property tag with a bad size");
        }
        let value = index + 3;
        let answered = match tag {
            GET_ARM_MEMORY if size >= 8 => {
                message[value] = 0;
                message[value + 1] = ARM_MEMORY_SIZE;
                8
            }
            GET_PHYSICAL_SIZE if size >= 8 => {
                message[value] = framebuffer.width;
                message[value + 1] = framebuffer.height;
                8
            }
            SET_PHYSICAL_SIZE if size >= 8 => {
                let (width, height) = (message[value], message[value + 1]);
                if width == 0 || height == 0 || width > 4096 || height > 4096 {
                    return Err("a screen of that size");
                }
                framebuffer.width = width;
                framebuffer.height = height;
                8
            }
            SET_VIRTUAL_SIZE if size >= 8 => {
                if (message[value], message[value + 1]) != (framebuffer.width, framebuffer.height) {
                    return Err("a virtual size other than the physical size");
                }
                8
            }
            SET_DEPTH if size >= 4 => {
                if message[value] != 32 {
                    return Err("depths other than 32 bits per pixel");
                }
                framebuffer.depth = 32;
                4
            }
            SET_PIXEL_ORDER if size >= 4 => {
                if message[value] > 1 {
                    return Err("pixel orders other than 0 and 1");
                }
                framebuffer.pixel_order = message[value];
                4
            }
            SET_VIRTUAL_OFFSET if size >= 8 => {
                if (message[value], message[value + 1]) != (0, 0) {
                    return Err("a virtual offset other than 0");
                }
                8
            }
            ALLOCATE_BUFFER if size >= 8 => {
                if framebuffer.depth != 32 {
                    return Err("a framebuffer of other than 32 bits per pixel");
                }
                let (address, bytes) = framebuffer.buffer();
                if address + bytes > GPU_MEMORY_END {
                    return Err("a framebuffer larger than the GPU's memory");
                }
                framebuffer.allocated = true;
                message[value] = FRAMEBUFFER_ADDRESS;
                message[value + 1] = bytes as u32;
                8
            }
            GET_PITCH if size >= 4 => {
                message[value] = framebuffer.pitch();
                4
            }
            _ => return Err("property tags other than the memory's and the framebuffer's"),
        };
        message[index + 2] = RESPONSE_SUCCESS | answered;
        index += 3 + size / 4;
    }
    message[1] = RESPONSE_SUCCESS;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_get_arm_memory() {
        let mut message = [32, 0, GET_ARM_MEMORY, 8, 0, 0, 0, 0];
        Mailbox::default().answer(&mut message).unwrap();
        assert_eq!(
            message,
            [32, RESPONSE_SUCCESS, GET_ARM_MEMORY, 8, RESPONSE_SUCCESS | 8, 0, ARM_MEMORY_SIZE, 0]
        );
    }

    #[test]
    fn rejects_unknown_tags_and_channels() {
        let mut message = [32, 0, 0x0001_0006, 8, 0, 0, 0, 0];
        assert!(Mailbox::default().answer(&mut message).is_err());
        assert!(Mailbox::default().write(WRITE, 0x1000 | 1).is_err());
    }

    #[test]
    fn sets_up_a_framebuffer_as_neos_asks() {
        let mut mailbox = Mailbox::default();
        let mut message = [32, 0, GET_PHYSICAL_SIZE, 8, 0, 0, 0, 0];
        mailbox.answer(&mut message).unwrap();
        assert_eq!(&message[4..7], [RESPONSE_SUCCESS | 8, 640, 480]);
        assert_eq!(mailbox.framebuffer(), None);

        #[rustfmt::skip]
        let mut message = [
            35 * 4, 0,
            SET_PHYSICAL_SIZE, 8, 0, 800, 600,
            SET_VIRTUAL_SIZE, 8, 0, 800, 600,
            SET_DEPTH, 4, 0, 32,
            SET_PIXEL_ORDER, 4, 0, 1,
            SET_VIRTUAL_OFFSET, 8, 0, 0, 0,
            ALLOCATE_BUFFER, 8, 0, 4096, 0,
            GET_PITCH, 4, 0, 0,
            0,
        ];
        mailbox.answer(&mut message).unwrap();
        assert_eq!(message[1], RESPONSE_SUCCESS);
        assert_eq!(&message[27..30], [RESPONSE_SUCCESS | 8, FRAMEBUFFER_ADDRESS, 800 * 600 * 4]);
        assert_eq!(&message[32..34], [RESPONSE_SUCCESS | 4, 800 * 4]);
        let framebuffer = mailbox.framebuffer().unwrap();
        assert_eq!((framebuffer.width, framebuffer.height, framebuffer.depth), (800, 600, 32));
    }

    #[test]
    fn refuses_what_it_does_not_model() {
        for (tag, words) in [
            (SET_DEPTH, [16, 0]),
            (SET_VIRTUAL_SIZE, [1280, 960]),
            (SET_VIRTUAL_OFFSET, [0, 16]),
            (SET_PHYSICAL_SIZE, [0, 480]),
        ] {
            let mut message = [32, 0, tag, 8, 0, words[0], words[1], 0];
            assert!(Mailbox::default().answer(&mut message).is_err());
        }
    }

    #[test]
    fn queues_the_answer() {
        let mut mailbox = Mailbox::default();
        assert_eq!(mailbox.read(STATUS), Ok(Some(EMPTY)));
        assert_eq!(mailbox.read(WRITE_STATUS), Ok(Some(0)));
        assert!(mailbox.read(READ).is_err());
        let Ok(Some(Request::Property { address })) = mailbox.write(WRITE, 0xC000_1008) else {
            panic!("expected a property request");
        };
        assert_eq!(address, 0x1000);
        assert_eq!(mailbox.read(STATUS), Ok(Some(0)));
        assert_eq!(mailbox.read(READ), Ok(Some(0xC000_1008)));
    }
}
