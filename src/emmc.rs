//! The Arasan EMMC controller and the SD card in its slot, as far as NeOS uses them
//! (ADR 0019).
//!
//! The controller follows the SD Host Controller Specification. Nebu runs each
//! command the moment it is written, as QEMU 8.2 does, so a command, and a block's
//! data, are ready before the next instruction. The card is a high-capacity card
//! held in an image file, which it reads and writes (ADR 0027). Every command,
//! register or state NeOS does not use halts Nebu.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

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

/// STATUS: the data lines are in use; there is room for a block to write; a block is
/// ready to read; a card is in the slot.
const DAT_INHIBIT: u32 = 1 << 1;
const WRITE_AVAILABLE: u32 = 1 << 10;
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

/// INTERRUPT: a command finished, a transfer finished (or a busy card let go), there
/// is room for a block to write, a block is ready to read, there is an error, and
/// the command timed out.
const CMD_DONE: u32 = 1 << 0;
const DATA_DONE: u32 = 1 << 1;
const WRITE_RDY: u32 = 1 << 4;
const READ_RDY: u32 = 1 << 5;
const ERROR: u32 = 1 << 15;
const CMD_TIMEOUT: u32 = 1 << 16;

/// CMDTM: the response type in bits 17:16, data present, and the direction.
const RESPONSE_BUSY: u32 = 3;
const DATA_PRESENT: u32 = 1 << 21;
const READ_FROM_CARD: u32 = 1 << 4;
/// CMDTM: a transfer of several blocks, and counting them down in BLKSIZECNT, which
/// NeOS uses together and only to write.
const MULTI_BLOCK: u32 = 1 << 5;
const BLOCK_COUNT_ENABLE: u32 = 1 << 1;
/// CMDTM's bits Nebu does not model: the command type (suspend, resume, abort), and
/// Auto CMD12 or CMD23.
const UNMODELLED_COMMAND_BITS: u32 = 0b11 << 22 | 0b11 << 2;

/// The only block size NeOS uses, and that a high-capacity card has.
pub const BLOCK: u64 = 512;
/// The largest standard-capacity card. Larger cards are high capacity.
pub const STANDARD_CAPACITY_LIMIT: u64 = 2 << 30;
/// The largest image Nebu takes: 32 GiB, the most an SDHC card holds. Larger cards
/// are SDXC, whose size the same CSD layout can hold, but NeOS's tests need none.
pub const HIGH_CAPACITY_LIMIT: u64 = 32 << 30;
/// The CSD gives a card's size in units of 512 KiB.
const SIZE_UNIT: u64 = 512 << 10;

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
    /// Taking the blocks of a write.
    Receiving,
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
            State::Receiving => 6,
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

/// A high-capacity card, held in an image file.
pub struct Card {
    image: File,
    blocks: u64,
    state: State,
    /// Whether the last command was CMD55, so this one is an application command.
    application: bool,
}

impl Card {
    /// A card holding `image`, whose size must be a whole number of 512 KiB, the
    /// unit the card-specific data gives its size in, more than 2 GiB and at most
    /// 32 GiB.
    pub fn new(image: File) -> Result<Card, String> {
        let size = image.metadata().map_err(|e| e.to_string())?.len();
        if size <= STANDARD_CAPACITY_LIMIT
            || size > HIGH_CAPACITY_LIMIT
            || !size.is_multiple_of(SIZE_UNIT)
        {
            return Err("the card image must be a whole number of 512 KiB, larger than 2 GiB \
                        and at most 32 GiB: Nebu models only high-capacity cards"
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
            // The status is the state the command found the card in, as for every
            // response.
            (false, 3, State::Identification) => {
                let status = self.status();
                self.state = State::Standby;
                Response::Short(RCA << 16 | status & 0xFF00)
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
            // CMD24 writes one block and CMD25 several, until CMD12. The data follows
            // the response.
            (false, 24 | 25, State::Transfer) if u64::from(argument) < self.blocks => {
                let status = self.status();
                self.state = State::Receiving;
                Response::Short(status)
            }
            (false, 24 | 25, State::Transfer) => return Err("a write past the end of the card"),
            (false, 12, State::Receiving) => {
                let status = self.status();
                self.state = State::Transfer;
                Response::Short(status)
            }
            (true, 41, _) => return Err("ACMD41 outside the idle state, or as an enquiry"),
            _ => return Err("an SD command in a state Nebu does not model"),
        };
        Ok(response)
    }

    /// Writes `buffer` to block `block`. The image file is written at once, so what
    /// NeOS writes is there the next time the image is used, as on a real card.
    fn write(&mut self, block: u64, buffer: &[u8; BLOCK as usize]) -> Result<(), &'static str> {
        if block >= self.blocks {
            return Err("a write past the end of the card");
        }
        self.image
            .seek(SeekFrom::Start(block * BLOCK))
            .and_then(|_| self.image.write_all(buffer))
            .map_err(|_| "writing the card image failed")
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
    /// The block being read or written, and how many of its bytes have been read.
    block: [u8; BLOCK as usize],
    read: Option<usize>,
    /// The write under way, if one is.
    writing: Option<Writing>,
}

/// A write the controller is taking from NeOS, a block at a time.
struct Writing {
    /// The card's block the next whole block goes to.
    block: u64,
    /// How many blocks are still to come, this one included.
    remaining: u32,
    /// How many bytes of this block have arrived.
    offset: usize,
    /// Whether it is CMD25's, which CMD12 ends, rather than CMD24's.
    multiple: bool,
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
            writing: None,
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
                let writing =
                    if self.writing.is_some() { DAT_INHIBIT | WRITE_AVAILABLE } else { 0 };
                let inserted = if self.card.is_some() { CARD_INSERTED } else { 0 };
                reading | writing | inserted
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
            DATA => self.write_data(value)?,
            CONTROL1 if value & SRST_HC != 0 => self.reset(),
            CONTROL1 => {
                // Resetting a line forgets what it was doing, and the interrupts
                // that recorded it, as QEMU does.
                if value & SRST_CMD != 0 {
                    self.interrupt &= !CMD_DONE;
                }
                if value & SRST_DATA != 0 {
                    self.read = None;
                    self.writing = None;
                    self.interrupt &= !(DATA_DONE | READ_RDY | WRITE_RDY);
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
        if self.writing.is_some() {
            return Err("a command sent while blocks are still being written");
        }
        if value & UNMODELLED_COMMAND_BITS != 0 {
            return Err("a command type or automatic command");
        }
        let index = value >> 24 & 0x3F;
        let response_type = value >> 16 & 0b11;
        let data = value & DATA_PRESENT != 0;
        let reading = value & READ_FROM_CARD != 0;
        let multiple = value & (MULTI_BLOCK | BLOCK_COUNT_ENABLE);
        if multiple != 0 && (multiple != MULTI_BLOCK | BLOCK_COUNT_ENABLE || reading || !data) {
            return Err("a multi-block transfer that is not a counted write");
        }
        if data && !reading && index != if multiple != 0 { 25 } else { 24 } {
            return Err("a write that is not CMD24 for one block or CMD25 for several");
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
            let count = self.blksizecnt >> 16;
            if u64::from(self.blksizecnt & 0xFFF) != BLOCK {
                return Err("a transfer of blocks of anything but 512 bytes");
            }
            if reading {
                if count != 1 {
                    return Err("a read of anything but one block");
                }
                card.read(u64::from(self.arg1), &mut self.block)?;
                self.read = Some(0);
                self.raise(READ_RDY);
            } else {
                let multiple = multiple != 0;
                let remaining = if multiple { count } else { 1 };
                if remaining == 0 {
                    return Err("a write of no blocks");
                }
                let block = u64::from(self.arg1);
                self.writing = Some(Writing { block, remaining, offset: 0, multiple });
                self.raise(WRITE_RDY);
            }
        }
        self.raise(CMD_DONE);
        // A card that holds the line busy has let go by now.
        if response_type == RESPONSE_BUSY {
            self.raise(DATA_DONE);
        }
        Ok(())
    }

    /// Takes the next word of the block being written. A whole block goes to the
    /// card at once, and the last finishes the transfer, since a busy card lets go
    /// at once.
    fn write_data(&mut self, word: u32) -> Result<(), &'static str> {
        let Some(writing) = &mut self.writing else {
            return Err("writing data with no write under way");
        };
        self.block[writing.offset..writing.offset + 4].copy_from_slice(&word.to_le_bytes());
        writing.offset += 4;
        if writing.offset < BLOCK as usize {
            return Ok(());
        }
        let block = writing.block;
        writing.block += 1;
        writing.remaining -= 1;
        writing.offset = 0;
        let (last, multiple) = (writing.remaining == 0, writing.multiple);
        let card = self.card.as_mut().ok_or("writing data with no card")?;
        card.write(block, &self.block)?;
        if !last {
            self.raise(WRITE_RDY);
            return Ok(());
        }
        self.writing = None;
        // CMD24's card is done with its one block; CMD25's waits for CMD12.
        if !multiple {
            card.state = State::Transfer;
        }
        self.raise(DATA_DONE);
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
        let image = File::options().read(true).write(true).open(&path).unwrap();
        let card = Card::new(image).unwrap();
        std::fs::remove_file(&path).unwrap();
        let mut emmc = Emmc::new(Some(card));
        emmc.write(CONTROL1, CLK_INTLEN, true).unwrap();
        emmc.write(CONTROL1, CLK_INTLEN | CLK_EN, true).unwrap();
        emmc.write(IRPT_MASK, 0xFFFF_0000 | READ_RDY | WRITE_RDY | DATA_DONE | CMD_DONE, true)
            .unwrap();
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

    /// Identifies and selects the card, leaving it in the transfer state.
    fn select(emmc: &mut Emmc) {
        command(emmc, 0, 0);
        command(emmc, 8 << 24 | 2 << 16, 0x1AA);
        command(emmc, 55 << 24 | 2 << 16, 0);
        command(emmc, 41 << 24 | 2 << 16, 0x4030_0000);
        command(emmc, 2 << 24 | 1 << 16, 0);
        command(emmc, 3 << 24 | 2 << 16, 0);
        command(emmc, 7 << 24 | 3 << 16, RCA << 16);
        emmc.write(INTERRUPT, DATA_DONE, true).unwrap();
    }

    /// Writes `blocks` blocks, each filled with its number, from block `first`.
    fn write_blocks(emmc: &mut Emmc, first: u32, blocks: u32) {
        let multiple = blocks > 1;
        emmc.write(BLKSIZECNT, blocks << 16 | 512, true).unwrap();
        let cmdtm = if multiple {
            25 << 24 | 2 << 16 | DATA_PRESENT | MULTI_BLOCK | BLOCK_COUNT_ENABLE
        } else {
            24 << 24 | 2 << 16 | DATA_PRESENT
        };
        command(emmc, cmdtm, first);
        for block in 0..blocks {
            assert_eq!(emmc.read(INTERRUPT).unwrap(), Some(WRITE_RDY));
            assert_eq!(emmc.read(STATUS).unwrap().unwrap() & WRITE_AVAILABLE, WRITE_AVAILABLE);
            emmc.write(INTERRUPT, WRITE_RDY, true).unwrap();
            for _ in 0..128 {
                emmc.write(DATA, first + block, true).unwrap();
            }
        }
        assert_eq!(emmc.read(INTERRUPT).unwrap(), Some(DATA_DONE));
        assert_eq!(emmc.read(STATUS).unwrap().unwrap() & DAT_INHIBIT, 0);
        emmc.write(INTERRUPT, DATA_DONE, true).unwrap();
        if multiple {
            command(emmc, 12 << 24 | 3 << 16, 0);
            emmc.write(INTERRUPT, DATA_DONE, true).unwrap();
        }
    }

    /// Reads block `block` and returns its first word.
    fn first_word(emmc: &mut Emmc, block: u32) -> u32 {
        emmc.write(BLKSIZECNT, 1 << 16 | 512, true).unwrap();
        command(emmc, 17 << 24 | 2 << 16 | DATA_PRESENT | READ_FROM_CARD, block);
        let word = emmc.read(DATA).unwrap().unwrap();
        for _ in 1..128 {
            emmc.read(DATA).unwrap();
        }
        emmc.write(INTERRUPT, READ_RDY | DATA_DONE, true).unwrap();
        word
    }

    #[test]
    fn writes_one_block_and_several() {
        let mut emmc = controller("write", &[]);
        select(&mut emmc);
        write_blocks(&mut emmc, 3, 1);
        assert_eq!(first_word(&mut emmc, 3), 3);
        write_blocks(&mut emmc, 10, 3);
        for block in 10..13 {
            assert_eq!(first_word(&mut emmc, block), block);
        }
        assert_eq!(first_word(&mut emmc, 13), 0);
        // A write past the end of the card halts.
        emmc.write(BLKSIZECNT, 1 << 16 | 512, true).unwrap();
        emmc.write(ARG1, 8 << 20, true).unwrap();
        assert_eq!(
            emmc.write(CMDTM, 24 << 24 | 2 << 16 | DATA_PRESENT, true),
            Err("a write past the end of the card")
        );
    }

    #[test]
    fn halts_on_a_command_in_the_middle_of_a_write() {
        let mut emmc = controller("write-midway", &[]);
        select(&mut emmc);
        emmc.write(BLKSIZECNT, 2 << 16 | 512, true).unwrap();
        command(&mut emmc, 25 << 24 | 2 << 16 | DATA_PRESENT | MULTI_BLOCK | BLOCK_COUNT_ENABLE, 0);
        assert_eq!(
            emmc.write(CMDTM, 13 << 24 | 2 << 16, true),
            Err("a command sent while blocks are still being written")
        );
        // Resetting the data line abandons it.
        let control1 = emmc.read(CONTROL1).unwrap().unwrap();
        emmc.write(CONTROL1, control1 | SRST_DATA, true).unwrap();
        assert_eq!(emmc.write(DATA, 0, true), Err("writing data with no write under way"));
    }

    #[test]
    fn identifies_the_card_and_reads_a_block() {
        let mut emmc = controller("read", b"NeOS");
        command(&mut emmc, 0, 0);
        assert_eq!(command(&mut emmc, 8 << 24 | 2 << 16, 0x1AA), 0x1AA);
        command(&mut emmc, 55 << 24 | 2 << 16, 0);
        assert_eq!(command(&mut emmc, 41 << 24 | 2 << 16, 0x4030_0000), OCR);
        command(&mut emmc, 2 << 24 | 1 << 16, 0);
        let published = command(&mut emmc, 3 << 24 | 2 << 16, 0);
        assert_eq!(published >> 16, RCA);
        // The state CMD3 found the card in: identification.
        assert_eq!(published >> 9 & 0xF, 2);
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

        // Resetting the data line forgets the read's interrupts.
        emmc.write(INTERRUPT, READ_RDY | DATA_DONE, true).unwrap();
        command(&mut emmc, 17 << 24 | 2 << 16 | DATA_PRESENT | READ_FROM_CARD, 1);
        assert_eq!(emmc.read(INTERRUPT).unwrap(), Some(READ_RDY));
        let control1 = emmc.read(CONTROL1).unwrap().unwrap();
        emmc.write(CONTROL1, control1 | SRST_DATA, true).unwrap();
        assert_eq!(emmc.read(INTERRUPT).unwrap(), Some(0));
        assert_eq!(emmc.read(STATUS).unwrap().unwrap() & DAT_INHIBIT, 0);
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
            emmc.write(CMDTM, 17 << 24 | 2 << 16, true),
            Err("an SD command in a state Nebu does not model")
        );
        assert_eq!(
            emmc.write(
                CMDTM,
                18 << 24 | 2 << 16 | DATA_PRESENT | READ_FROM_CARD | MULTI_BLOCK,
                true
            ),
            Err("a multi-block transfer that is not a counted write")
        );
        assert_eq!(
            emmc.write(CMDTM, 25 << 24 | 2 << 16 | DATA_PRESENT | MULTI_BLOCK, true),
            Err("a multi-block transfer that is not a counted write")
        );
        assert_eq!(
            emmc.write(CMDTM, 24 << 24 | 2 << 16 | DATA_PRESENT | 1 << 2, true),
            Err("a command type or automatic command")
        );
        assert_eq!(
            emmc.write(CMDTM, 25 << 24 | 2 << 16 | DATA_PRESENT, true),
            Err("a write that is not CMD24 for one block or CMD25 for several")
        );
        assert_eq!(emmc.write(0x28, 0, true), Ok(None));
        emmc.write(CONTROL1, 0, true).unwrap();
        assert_eq!(emmc.write(CMDTM, 0, true), Err("a command sent with the card's clock off"));
    }
}
