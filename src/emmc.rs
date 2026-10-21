//! The Arasan EMMC controller and the SD card in its slot, as far as NeOS uses them
//! (ADR 0019).
//!
//! The controller follows the SD Host Controller Specification. Nebu runs each
//! command the moment it is written, as QEMU 8.2 does, so a command, and a block's
//! data, are ready before the next instruction. The card is a high-capacity card
//! read from an image file, and is only ever read. Every command, register or state
//! NeOS does not use halts Nebu.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

/// The registers NeOS uses, as offsets from the controller's base at 0x3F30_0000.
const BLKSIZECNT: u64 = 0x04;
const ARG1: u64 = 0x08;
const CMDTM: u64 = 0x0C;
const RESP0: u64 = 0x10;
const RESP3: u64 = 0x1C;
const DATA: u64 = 0x20;
const STATUS: u64 = 0x24;
const CONTROL1: u64 = 0x2C;
const INTERRUPT: u64 = 0x30;
const IRPT_MASK: u64 = 0x34;
const IRPT_EN: u64 = 0x38;

/// STATUS: the data lines are in use; a block is ready to read; a card is in the
/// slot.
const DAT_INHIBIT: u32 = 1 << 1;
const READ_AVAILABLE: u32 = 1 << 11;
const CARD_INSERTED: u32 = 1 << 16;

/// CONTROL1: the internal clock is on, has settled, and reaches the card; and the
/// three resets, which finish at once.
const CLK_INTLEN: u32 = 1 << 0;
const CLK_STABLE: u32 = 1 << 1;
const CLK_EN: u32 = 1 << 2;
const SRST_HC: u32 = 1 << 24;
const SRST_CMD: u32 = 1 << 25;
const SRST_DATA: u32 = 1 << 26;

/// INTERRUPT: a command finished, a transfer finished (or a busy card let go), a
/// block is ready to read, there is an error, and the command timed out.
const CMD_DONE: u32 = 1 << 0;
const DATA_DONE: u32 = 1 << 1;
const READ_RDY: u32 = 1 << 5;
const ERROR: u32 = 1 << 15;
const CMD_TIMEOUT: u32 = 1 << 16;

/// CMDTM: the response type in bits 17:16, data present, and the direction.
const RESPONSE_BUSY: u32 = 3;
const DATA_PRESENT: u32 = 1 << 21;
const READ_FROM_CARD: u32 = 1 << 4;

/// The only block size NeOS uses, and that a high-capacity card has.
pub const BLOCK: u64 = 512;
/// The largest standard-capacity card. Larger cards are high capacity.
pub const STANDARD_CAPACITY_LIMIT: u64 = 2 << 30;

/// The relative card address the card publishes, as QEMU's first card does.
const RCA: u32 = 0x4567;
/// The OCR the card reports once it has powered up: 2.7 to 3.6 V, high capacity,
/// and ready.
const OCR: u32 = 0x00FF_8000 | 1 << 30 | 1 << 31;

/// The states of the card that NeOS takes it through.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Idle,
    Ready,
    Identification,
    Standby,
    Transfer,
}

impl State {
    /// Its number, as an R1 response reports it in bits 12:9.
    fn number(self) -> u32 {
        match self {
            State::Idle => 0,
            State::Ready => 1,
            State::Identification => 2,
            State::Standby => 3,
            State::Transfer => 4,
        }
    }
}

/// What a card sends back for a command.
enum Response {
    None,
    Short(u32),
    /// Bits 127:8 of a 136-bit response, as the controller stores them.
    Long(u128),
}

/// A high-capacity card, read from an image file.
pub struct Card {
    image: File,
    blocks: u64,
    state: State,
    /// Whether the last command was CMD55, so this one is an application command.
    application: bool,
}

impl Card {
    /// A card holding `image`, whose size must be a whole number of blocks and more
    /// than 2 GiB.
    pub fn new(image: File) -> Result<Card, String> {
        let size = image.metadata().map_err(|e| e.to_string())?.len();
        if size <= STANDARD_CAPACITY_LIMIT || !size.is_multiple_of(BLOCK) {
            return Err("the card image must be a whole number of 512-byte blocks and larger \
                        than 2 GiB: Nebu models only high-capacity cards"
                .into());
        }
        Ok(Card { image, blocks: size / BLOCK, state: State::Idle, application: false })
    }

    /// The card status an R1 response carries: its state, ready for data, and
    /// whether the next command is an application command.
    fn status(&self) -> u32 {
        self.state.number() << 9 | 1 << 8 | u32::from(self.application) << 5
    }

    /// The card-specific data, version 2.0: its bits 127:8.
    fn csd(&self) -> u128 {
        let size = u128::from(self.blocks / 1024 - 1);
        let csd: u128 = 1 << 126 // CSD_STRUCTURE: version 2.0
            | 0x0E << 112 // TAAC: 1 ms
            | 0x32 << 96 // TRAN_SPEED: 25 MHz
            | 0x5B5 << 84 // CCC: the command classes
            | 9 << 80 // READ_BL_LEN: 512 bytes
            | size << 48 // C_SIZE
            | 1 << 46 // ERASE_BLK_EN
            | 0x7F << 39 // SECTOR_SIZE
            | 2 << 26 // R2W_FACTOR
            | 9 << 22 // WRITE_BL_LEN: 512 bytes
            | 1; // the last bit is always 1
        csd >> 8
    }

    /// Runs command `index` with `argument`. Returns the response, or what NeOS did
    /// that Nebu does not model.
    fn command(&mut self, index: u32, argument: u32) -> Result<Response, &'static str> {
        let application = std::mem::take(&mut self.application);
        let response = match (application, index, self.state) {
            (false, 0, _) => {
                self.state = State::Idle;
                Response::None
            }
            (false, 8, State::Idle) => Response::Short(argument & 0xFFF),
            (false, 55, _) => {
                self.application = true;
                Response::Short(self.status())
            }
            // ACMD41 with a voltage window powers the card up at once, as QEMU's does.
            (true, 41, State::Idle) if argument & 0x00FF_FFFF != 0 => {
                self.state = State::Ready;
                Response::Short(OCR)
            }
            (false, 2, State::Ready) => {
                self.state = State::Identification;
                // A made-up manufacturer, product name "NEBU" and serial number.
                Response::Long(0x4E_4542_5500_0000_0001 << 32)
            }
            (false, 3, State::Identification) => {
                self.state = State::Standby;
                Response::Short(RCA << 16 | self.status() & 0xFF00)
            }
            (false, 9, State::Standby) if argument >> 16 == RCA => Response::Long(self.csd()),
            (false, 7, State::Standby) if argument >> 16 == RCA => {
                let status = self.status();
                self.state = State::Transfer;
                Response::Short(status)
            }
            (false, 17, State::Transfer) if u64::from(argument) < self.blocks => {
                Response::Short(self.status())
            }
            (false, 17, State::Transfer) => return Err("CMD17 past the end of the card"),
            (true, 41, _) => return Err("ACMD41 outside the idle state, or as an enquiry"),
            _ => return Err("an SD command in a state Nebu does not model"),
        };
        Ok(response)
    }

    /// Reads block `block` into `buffer`.
    fn read(&mut self, block: u64, buffer: &mut [u8; BLOCK as usize]) -> Result<(), &'static str> {
        self.image
            .seek(SeekFrom::Start(block * BLOCK))
            .and_then(|_| self.image.read_exact(buffer))
            .map_err(|_| "reading the card image failed")
    }
}

/// The controller's state.
pub struct Emmc {
    card: Option<Card>,
    blksizecnt: u32,
    arg1: u32,
    response: [u32; 4],
    control1: u32,
    /// Which interrupts have happened, of those IRPT_MASK lets it record.
    interrupt: u32,
    irpt_mask: u32,
    irpt_en: u32,
    /// The block being read, and how many of its bytes have been read.
    block: [u8; BLOCK as usize],
    read: Option<usize>,
}

impl Emmc {
    /// The controller after reset, with `card` in the slot, if there is one.
    pub fn new(card: Option<Card>) -> Emmc {
        Emmc {
            card,
            blksizecnt: 0,
            arg1: 0,
            response: [0; 4],
            control1: 0,
            interrupt: 0,
            irpt_mask: 0,
            irpt_en: 0,
            block: [0; BLOCK as usize],
            read: None,
        }
    }

    /// Resets every register, but not the card.
    fn reset(&mut self) {
        let card = self.card.take();
        *self = Emmc::new(card);
    }

    /// Records `bits` in the interrupt status, as far as IRPT_MASK lets it.
    fn raise(&mut self, bits: u32) {
        self.interrupt |= bits & self.irpt_mask;
    }

    /// Whether the controller is raising its interrupt, 62.
    pub fn interrupt(&self) -> bool {
        self.status_register() & self.irpt_en != 0
    }

    /// INTERRUPT as it reads: bit 15 is set while any error is.
    fn status_register(&self) -> u32 {
        let error = if self.interrupt & 0xFFFF_0000 != 0 { ERROR } else { 0 };
        self.interrupt | error
    }

    /// Reads the register at `offset`. `Ok(None)` is a register Nebu does not
    /// implement.
    pub fn read(&mut self, offset: u64) -> Result<Option<u32>, &'static str> {
        Ok(Some(match offset {
            BLKSIZECNT => self.blksizecnt,
            ARG1 => self.arg1,
            RESP0..=RESP3 if offset.is_multiple_of(4) => {
                self.response[((offset - RESP0) / 4) as usize]
            }
            DATA => return self.read_data().map(Some),
            STATUS => {
                let reading = if self.read.is_some() { DAT_INHIBIT | READ_AVAILABLE } else { 0 };
                let inserted = if self.card.is_some() { CARD_INSERTED } else { 0 };
                reading | inserted
            }
            CONTROL1 => self.control1,
            INTERRUPT => self.status_register(),
            IRPT_MASK => self.irpt_mask,
            IRPT_EN => self.irpt_en,
            _ => return Ok(None),
        }))
    }

    /// Writes the register at `offset`. `connected` says whether the card's pins are
    /// switched to this controller. `Ok(None)` is a register Nebu does not
    /// implement.
    pub fn write(
        &mut self,
        offset: u64,
        value: u32,
        connected: bool,
    ) -> Result<Option<()>, &'static str> {
        match offset {
            BLKSIZECNT => self.blksizecnt = value,
            ARG1 => self.arg1 = value,
            CMDTM => self.command(value, connected)?,
            CONTROL1 if value & SRST_HC != 0 => self.reset(),
            CONTROL1 => {
                if value & (SRST_CMD | SRST_DATA) != 0 {
                    self.read = None;
                }
                let value = value & !(SRST_CMD | SRST_DATA | CLK_STABLE);
                let stable = if value & CLK_INTLEN != 0 { CLK_STABLE } else { 0 };
                self.control1 = value | stable;
            }
            // Writing 1 clears a bit. Bit 15 follows the errors.
            INTERRUPT => self.interrupt &= !value,
            // Turning off a bit here forgets it, as QEMU does.
            IRPT_MASK => {
                self.irpt_mask = value;
                self.interrupt &= value;
            }
            IRPT_EN => self.irpt_en = value,
            _ => return Ok(None),
        }
        Ok(Some(()))
    }

    /// Sends the command written to CMDTM, with the argument in ARG1.
    fn command(&mut self, value: u32, connected: bool) -> Result<(), &'static str> {
        if self.control1 & (CLK_INTLEN | CLK_STABLE | CLK_EN) != CLK_INTLEN | CLK_STABLE | CLK_EN {
            return Err("a command sent with the card's clock off");
        }
        if self.read.is_some() {
            return Err("a command sent while a block is still being read");
        }
        let index = value >> 24 & 0x3F;
        let response_type = value >> 16 & 0b11;
        let data = value & DATA_PRESENT != 0;
        if data && value & READ_FROM_CARD == 0 {
            return Err("writing to the card");
        }
        let card = match &mut self.card {
            Some(card) if connected => card,
            // No card answers: the command times out, as in QEMU.
            _ => {
                if response_type != 0 {
                    self.raise(CMD_TIMEOUT);
                }
                self.raise(CMD_DONE);
                return Ok(());
            }
        };
        let response = card.command(index, self.arg1)?;
        match (response_type, response) {
            (0, Response::None) => {}
            (1, Response::Long(bits)) => {
                self.response =
                    [bits as u32, (bits >> 32) as u32, (bits >> 64) as u32, (bits >> 96) as u32];
            }
            (2 | 3, Response::Short(word)) => self.response = [word, 0, 0, 0],
            _ => return Err("a command with the wrong response type"),
        }
        if data {
            if u64::from(self.blksizecnt & 0xFFF) != BLOCK || self.blksizecnt >> 16 != 1 {
                return Err("a transfer of anything but one 512-byte block");
            }
            card.read(u64::from(self.arg1), &mut self.block)?;
            self.read = Some(0);
            self.raise(READ_RDY);
        }
        self.raise(CMD_DONE);
        // A card that holds the line busy has let go by now.
        if response_type == RESPONSE_BUSY {
            self.raise(DATA_DONE);
        }
        Ok(())
    }

    /// Reads the next word of the block being read.
    fn read_data(&mut self) -> Result<u32, &'static str> {
        let Some(offset) = self.read else {
            return Err("reading data with no block ready");
        };
        let word = u32::from_le_bytes([
            self.block[offset],
            self.block[offset + 1],
            self.block[offset + 2],
            self.block[offset + 3],
        ]);
        if offset + 4 == BLOCK as usize {
            self.read = None;
            self.raise(DATA_DONE);
        } else {
            self.read = Some(offset + 4);
        }
        Ok(word)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A controller with a 4 GiB sparse card whose block 1 starts with `bytes`.
    fn controller(name: &str, bytes: &[u8]) -> Emmc {
        let path =
            std::env::temp_dir().join(format!("nebu-emmc-{name}-{}.img", std::process::id()));
        let mut file = File::create(&path).unwrap();
        file.set_len(4 << 30).unwrap();
        file.seek(SeekFrom::Start(BLOCK)).unwrap();
        file.write_all(bytes).unwrap();
        let card = Card::new(File::open(&path).unwrap()).unwrap();
        std::fs::remove_file(&path).unwrap();
        let mut emmc = Emmc::new(Some(card));
        emmc.write(CONTROL1, CLK_INTLEN, true).unwrap();
        emmc.write(CONTROL1, CLK_INTLEN | CLK_EN, true).unwrap();
        emmc.write(IRPT_MASK, 0xFFFF_0000 | READ_RDY | DATA_DONE | CMD_DONE, true).unwrap();
        emmc
    }

    /// Sends a command and returns RESP0, after checking it finished with no error.
    fn command(emmc: &mut Emmc, cmdtm: u32, argument: u32) -> u32 {
        emmc.write(ARG1, argument, true).unwrap();
        emmc.write(CMDTM, cmdtm, true).unwrap();
        let status = emmc.read(INTERRUPT).unwrap().unwrap();
        assert_eq!(status & (ERROR | CMD_DONE), CMD_DONE);
        emmc.write(INTERRUPT, CMD_DONE, true).unwrap();
        emmc.read(RESP0).unwrap().unwrap()
    }

    #[test]
    fn identifies_the_card_and_reads_a_block() {
        let mut emmc = controller("read", b"NeOS");
        command(&mut emmc, 0, 0);
        assert_eq!(command(&mut emmc, 8 << 24 | 2 << 16, 0x1AA), 0x1AA);
        command(&mut emmc, 55 << 24 | 2 << 16, 0);
        assert_eq!(command(&mut emmc, 41 << 24 | 2 << 16, 0x4030_0000), OCR);
        command(&mut emmc, 2 << 24 | 1 << 16, 0);
        assert_eq!(command(&mut emmc, 3 << 24 | 2 << 16, 0) >> 16, RCA);
        command(&mut emmc, 9 << 24 | 1 << 16, RCA << 16);
        // C_SIZE, in bits 61:40 of the registers: 4 GiB is 8192 units of 512 KiB.
        assert_eq!(emmc.read(RESP0 + 4).unwrap().unwrap() >> 8 & 0x3F_FFFF, 8191);
        assert_eq!(emmc.read(RESP3).unwrap().unwrap() >> 22 & 0b11, 1);
        command(&mut emmc, 7 << 24 | 3 << 16, RCA << 16);
        assert_eq!(emmc.read(INTERRUPT).unwrap(), Some(DATA_DONE));
        emmc.write(INTERRUPT, DATA_DONE, true).unwrap();

        emmc.write(BLKSIZECNT, 1 << 16 | 512, true).unwrap();
        command(&mut emmc, 17 << 24 | 2 << 16 | DATA_PRESENT | READ_FROM_CARD, 1);
        assert_eq!(emmc.read(INTERRUPT).unwrap(), Some(READ_RDY));
        assert_eq!(emmc.read(DATA).unwrap(), Some(u32::from_le_bytes(*b"NeOS")));
        for _ in 1..128 {
            emmc.read(DATA).unwrap();
        }
        assert_eq!(emmc.read(INTERRUPT).unwrap(), Some(READ_RDY | DATA_DONE));
        assert_eq!(emmc.read(DATA), Err("reading data with no block ready"));
    }

    #[test]
    fn times_out_when_the_pins_go_elsewhere() {
        let mut emmc = controller("pins", &[]);
        emmc.write(CMDTM, 8 << 24 | 2 << 16, false).unwrap();
        assert_eq!(emmc.read(INTERRUPT).unwrap(), Some(CMD_TIMEOUT | ERROR | CMD_DONE));
        emmc.write(IRPT_EN, CMD_TIMEOUT, false).unwrap();
        assert!(emmc.interrupt());
    }

    #[test]
    fn halts_on_what_it_does_not_model() {
        let mut emmc = controller("halts", &[]);
        assert_eq!(
            emmc.write(CMDTM, 24 << 24 | 2 << 16 | DATA_PRESENT, true),
            Err("writing to the card")
        );
        assert_eq!(
            emmc.write(CMDTM, 17 << 24 | 2 << 16, true),
            Err("an SD command in a state Nebu does not model")
        );
        assert_eq!(emmc.write(0x28, 0, true), Ok(None));
        emmc.write(CONTROL1, 0, true).unwrap();
        assert_eq!(emmc.write(CMDTM, 0, true), Err("a command sent with the card's clock off"));
    }
}
