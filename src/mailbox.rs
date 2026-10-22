//! The VideoCore mailbox, as far as NeOS uses it: asking the firmware how much memory
//! the CPU has (ADR 0011).
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

/// The mailbox registers NeOS uses.
#[derive(Default)]
pub struct Mailbox {
    answers: VecDeque<u32>,
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

/// Answers a property message in place. `message` holds its words, starting with its
/// size in bytes.
pub fn answer(message: &mut [u32]) -> Result<(), &'static str> {
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
        match tag {
            GET_ARM_MEMORY if size >= 8 => {
                message[index + 2] = RESPONSE_SUCCESS | 8;
                message[index + 3] = 0;
                message[index + 4] = ARM_MEMORY_SIZE;
            }
            _ => return Err("property tags other than GET_ARM_MEMORY"),
        }
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
        answer(&mut message).unwrap();
        assert_eq!(
            message,
            [32, RESPONSE_SUCCESS, GET_ARM_MEMORY, 8, RESPONSE_SUCCESS | 8, 0, ARM_MEMORY_SIZE, 0]
        );
    }

    #[test]
    fn rejects_unknown_tags_and_channels() {
        let mut message = [32, 0, 0x0001_0006, 8, 0, 0, 0, 0];
        assert!(answer(&mut message).is_err());
        assert!(Mailbox::default().write(WRITE, 0x1000 | 1).is_err());
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
