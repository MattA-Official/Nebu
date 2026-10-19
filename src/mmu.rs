//! Address translation: the stage 1 translation of the EL1&0 regime (ADR 0009).
//!
//! When SCTLR_EL1.M is set, every instruction fetch and data access at EL1 or EL0 goes
//! through the page tables that TTBR0_EL1 and TTBR1_EL1 point to. Nebu walks the
//! tables afresh on every access instead of caching translations in a TLB, so TLB
//! maintenance instructions have nothing to do.
//!
//! Only what NeOS uses is supported: 4 KiB pages and 39-bit virtual addresses in both
//! halves, so every walk starts at level 1. Anything else stops Nebu with an
//! explanation. The descriptor formats are in the Arm Architecture Reference Manual
//! (Arm DDI 0487), section D8.3.

use crate::bus::{Bus, BusError};

/// SCTLR_ELx.M: the MMU is on.
pub const SCTLR_M: u64 = 1;

/// The registers that control translation.
#[derive(Clone, Copy, Debug, Default)]
pub struct Registers {
    pub sctlr: u64,
    pub tcr: u64,
    pub mair: u64,
    pub ttbr0: u64,
    pub ttbr1: u64,
}

/// What kind of access is being translated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Fetch,
    Read,
    Write,
}

/// Where an access goes in physical memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Translation {
    /// The physical address.
    pub address: u64,
    /// Whether the memory is Device memory, which must be accessed with aligned
    /// accesses. Otherwise it is Normal memory.
    pub device: bool,
}

/// Why a translation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The access faults, and the CPU takes an abort. `status` is the fault status
    /// code for ESR_EL1 (its bits 5 to 0).
    Abort { status: u8 },
    /// Reading a page table entry failed.
    Table { address: u64, error: BusError },
    /// The tables use a feature Nebu does not model.
    Unsupported(&'static str),
}

/// Fault status codes: each kind of fault, plus the level it happened at.
const ADDRESS_SIZE_FAULT: u8 = 0b00_0000;
const TRANSLATION_FAULT: u8 = 0b00_0100;
const ACCESS_FLAG_FAULT: u8 = 0b00_1000;
const PERMISSION_FAULT: u8 = 0b00_1100;

/// The only translation table shape Nebu supports, as TCR_EL1 describes it:
/// T0SZ = T1SZ = 25 (39-bit addresses), TG0 = 0b00 and TG1 = 0b10 (4 KiB pages).
const VIRTUAL_BITS: u32 = 39;
const TCR_SHAPE_MASK: u64 = 0b11 << 30 | 0x3F << 16 | 0b11 << 14 | 0x3F;
const TCR_SHAPE: u64 = 0b10 << 30 | 25 << 16 | 25;
/// TCR_EL1.EPD0 and EPD1: don't walk TTBR0's or TTBR1's tables.
const TCR_EPD0: u64 = 1 << 7;
const TCR_EPD1: u64 = 1 << 23;

/// The output address bits of a table descriptor, and of TTBRn's base address.
const ADDRESS: u64 = 0x0000_FFFF_FFFF_F000;
/// Bits 63 to 59 of a table descriptor: APTable, UXNTable/XNTable, PXNTable and NSTable.
const TABLE_ATTRIBUTES: u64 = 0b1_1111 << 59;

/// Translates `address`, accessed by `access` at exception level `el`.
pub fn translate(
    bus: &mut Bus,
    registers: &Registers,
    address: u64,
    access: Access,
    el: u8,
) -> Result<Translation, Error> {
    // EL2 has its own translation regime, which Nebu does not model: NeOS only runs
    // there with the MMU off.
    if el >= 2 || registers.sctlr & SCTLR_M == 0 {
        // With the MMU off, data accesses are to Device memory, and instructions are
        // fetched as if from Normal memory.
        return Ok(Translation { address, device: access != Access::Fetch });
    }
    if registers.tcr & TCR_SHAPE_MASK != TCR_SHAPE {
        return Err(Error::Unsupported("page tables other than 39-bit with 4 KiB pages"));
    }

    // The top bits select the half: all zeros for TTBR0, all ones for TTBR1. Anything
    // else is outside both, and faults at level 0.
    let top = address >> VIRTUAL_BITS;
    let (ttbr, disabled) = match top {
        0 => (registers.ttbr0, registers.tcr & TCR_EPD0 != 0),
        top if top == u64::MAX >> VIRTUAL_BITS => (registers.ttbr1, registers.tcr & TCR_EPD1 != 0),
        _ => (0, true),
    };
    if disabled {
        return Err(Error::Abort { status: TRANSLATION_FAULT });
    }

    let physical_bits = physical_address_bits(registers.tcr)?;
    let mut table = ttbr & ADDRESS;
    for level in 1..=3u8 {
        let shift = 12 + 9 * u32::from(3 - level);
        let index = (address >> shift) & 0x1FF;
        let entry_address = table + index * 8;
        let entry = bus
            .read(entry_address, 8)
            .map_err(|error| Error::Table { address: entry_address, error })?;

        let leaf = match (entry & 0b11, level) {
            (0b00 | 0b10, _) | (0b01, 3) => {
                return Err(Error::Abort { status: TRANSLATION_FAULT | level });
            }
            (0b11, 1 | 2) => false,
            _ => true,
        };
        // A block's output address only has the bits above the part of the address
        // it maps; a table's or a page's is 4 KiB-aligned.
        let output = if leaf { entry & ADDRESS & !((1 << shift) - 1) } else { entry & ADDRESS };
        if output >> physical_bits != 0 {
            return Err(Error::Abort { status: ADDRESS_SIZE_FAULT | level });
        }
        if !leaf {
            if entry & TABLE_ATTRIBUTES != 0 {
                return Err(Error::Unsupported("table descriptor attributes (APTable and others)"));
            }
            table = output;
            continue;
        }

        if entry & 1 << 10 == 0 {
            return Err(Error::Abort { status: ACCESS_FLAG_FAULT | level });
        }
        if !permitted(entry, access, el) {
            return Err(Error::Abort { status: PERMISSION_FAULT | level });
        }
        let attribute = registers.mair >> (8 * (entry >> 2 & 0b111)) & 0xFF;
        let device = attribute & 0xF0 == 0;
        if device && access == Access::Fetch {
            return Err(Error::Unsupported("instruction fetch from Device memory"));
        }
        return Ok(Translation { address: output | address & ((1 << shift) - 1), device });
    }
    unreachable!("level 3 entries are always leaves or faults")
}

/// Whether a leaf entry's AP, PXN and UXN bits allow `access` at `el`.
fn permitted(entry: u64, access: Access, el: u8) -> bool {
    // AP[2] (bit 7) makes the memory read-only; AP[1] (bit 6) gives EL0 access too.
    let read_only = entry & 1 << 7 != 0;
    let user = entry & 1 << 6 != 0;
    let kernel_no_execute = entry & 1 << 53 != 0;
    let user_no_execute = entry & 1 << 54 != 0;
    match (access, el) {
        (Access::Read, 0) => user,
        (Access::Read, _) => true,
        (Access::Write, 0) => user && !read_only,
        (Access::Write, _) => !read_only,
        (Access::Fetch, 0) => !user_no_execute,
        // EL1 can never execute memory that EL0 can write.
        (Access::Fetch, _) => !kernel_no_execute && !(user && !read_only),
    }
}

/// The size of physical addresses, from TCR_EL1.IPS.
fn physical_address_bits(tcr: u64) -> Result<u32, Error> {
    match tcr >> 32 & 0b111 {
        0b000 => Ok(32),
        0b001 => Ok(36),
        0b010 => Ok(40),
        // The Cortex-A53 supports 40 bits at most.
        _ => Err(Error::Unsupported("physical addresses of more than 40 bits")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pl011::Pl011;

    const WINDOW: u64 = 0xFFFF_FF80_0000_0000;
    /// Normal memory (MAIR type 1), inner shareable, accessed, EL0 can't execute.
    const NORMAL: u64 = 1 << 2 | 0b11 << 8 | 1 << 10 | 1 << 54;

    fn bus() -> Bus {
        Bus::new(Pl011::new(Box::new(std::io::sink()), std::sync::mpsc::channel().1))
    }

    /// Tables at 0x10000 (level 1), 0x11000 (level 2) and 0x12000 (level 3), used by
    /// both halves. They map 0x200000 to 0x400000 as a 2 MiB block, and the page at
    /// 0x5000 read-only. TTBR0 is switched off.
    fn setup() -> (Bus, Registers) {
        let mut bus = bus();
        bus.write(0x10000, 8, 0x11000 | 0b11).unwrap();
        bus.write(0x11000, 8, 0x12000 | 0b11).unwrap();
        bus.write(0x11008, 8, 0x200000 | NORMAL | 0b01).unwrap();
        bus.write(0x12028, 8, 0x5000 | NORMAL | 1 << 7 | 1 << 53 | 0b11).unwrap();
        let registers = Registers {
            sctlr: SCTLR_M,
            tcr: TCR_SHAPE | TCR_EPD0,
            mair: 0xFF04,
            ttbr0: 0x10000,
            ttbr1: 0x10000,
        };
        (bus, registers)
    }

    #[test]
    fn walks_blocks_and_pages() {
        let (mut bus, registers) = setup();
        let translate =
            |bus: &mut Bus, address, access| translate(bus, &registers, address, access, 1);
        assert_eq!(
            translate(&mut bus, WINDOW + 0x212345, Access::Write),
            Ok(Translation { address: 0x212345, device: false })
        );
        assert_eq!(
            translate(&mut bus, WINDOW + 0x5008, Access::Read),
            Ok(Translation { address: 0x5008, device: false })
        );
        // Read-only, and not executable at EL1.
        let permission = Err(Error::Abort { status: PERMISSION_FAULT | 3 });
        assert_eq!(translate(&mut bus, WINDOW + 0x5008, Access::Write), permission);
        assert_eq!(translate(&mut bus, WINDOW + 0x5008, Access::Fetch), permission);
        // Nothing at 0x6000, or at 0x40000000 (level 1 entry 1).
        let missing = |level| Err(Error::Abort { status: TRANSLATION_FAULT | level });
        assert_eq!(translate(&mut bus, WINDOW + 0x6000, Access::Read), missing(3));
        assert_eq!(translate(&mut bus, WINDOW + 0x4000_0000, Access::Read), missing(1));
        // TTBR0 is off, and some addresses are in neither half.
        assert_eq!(translate(&mut bus, 0x5000, Access::Read), missing(0));
        assert_eq!(translate(&mut bus, 0x0000_8000_0000_0000, Access::Read), missing(0));
    }

    #[test]
    fn el0_needs_ap1() {
        let (mut bus, registers) = setup();
        let permission = Err(Error::Abort { status: PERMISSION_FAULT | 2 });
        assert_eq!(translate(&mut bus, &registers, WINDOW + 0x200000, Access::Read, 0), permission);
    }

    #[test]
    fn off_means_identity() {
        let (mut bus, mut registers) = setup();
        registers.sctlr = 0;
        assert_eq!(
            translate(&mut bus, &registers, 0x3F20_1000, Access::Write, 1),
            Ok(Translation { address: 0x3F20_1000, device: true })
        );
    }
}
