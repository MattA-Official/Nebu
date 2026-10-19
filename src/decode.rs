//! Decoding A64 instruction words (ADR 0004).
//!
//! `decode` turns a 32-bit word into an [`Instruction`] without executing anything.
//! The structure follows the encoding index in the Arm Architecture Reference Manual
//! for A-profile (Arm DDI 0487), chapter C4: the word is split into ever smaller
//! groups until it names exactly one instruction.
//!
//! Only instructions NeOS uses are decoded (ADR 0001). The unit of support is one
//! instruction page of the Arm ARM, such as "STR (immediate)": every form on a
//! supported page is decoded, and anything else is an error.

use std::fmt;

/// The size of the registers an instruction operates on: the `sf` bit, or the
/// `size` field for loads and stores.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    /// 32-bit, using the W registers.
    W32,
    /// 64-bit, using the X registers.
    W64,
}

impl Width {
    fn from_bit(bit: u32) -> Self {
        if bit == 0 { Width::W32 } else { Width::W64 }
    }

    /// The number of bits.
    pub fn bits(self) -> u32 {
        match self {
            Width::W32 => 32,
            Width::W64 => 64,
        }
    }

    /// The number of bytes.
    pub fn bytes(self) -> u64 {
        u64::from(self.bits() / 8)
    }

    /// A mask of all the bits in a register of this width.
    pub fn mask(self) -> u64 {
        match self {
            Width::W32 => 0xFFFF_FFFF,
            Width::W64 => u64::MAX,
        }
    }
}

/// How a load or store updates its base register.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Index {
    /// Access `base + offset`; leave the base unchanged.
    Offset,
    /// Access `base + offset`, then set the base to that address.
    Pre,
    /// Access `base`, then add the offset to the base.
    Post,
}

/// The shift applied to the second register in a shifted-register instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shift {
    /// Logical shift left.
    Lsl,
    /// Logical shift right.
    Lsr,
    /// Arithmetic shift right.
    Asr,
    /// Rotate right (logical instructions only).
    Ror,
}

/// How a register offset is extended before it is added to the base address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extend {
    /// Zero-extend the low 32 bits.
    Uxtw,
    /// Use all 64 bits unchanged (written `lsl` in assembler).
    Lsl,
    /// Sign-extend the low 32 bits.
    Sxtw,
    /// Use all 64 bits unchanged (written `sxtx` in assembler).
    Sxtx,
}

/// A system register that `MRS` or `MSR` can name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemRegister {
    /// The multiprocessor affinity register: which core this is.
    MpidrEl1,
    /// The current exception level.
    CurrentEl,
    /// EL1's system control register: the MMU, caches and alignment checks.
    SctlrEl1,
    /// EL1's saved program status register, written when an exception is taken.
    SpsrEl1,
    /// EL1's exception link register: where `eret` returns to.
    ElrEl1,
    /// The stack pointer of EL0.
    SpEl0,
    /// EL1's exception syndrome register: why the last exception happened.
    EsrEl1,
    /// EL1's fault address register.
    FarEl1,
    /// EL1's vector base address register: where its exception vectors are.
    VbarEl1,
    /// The hypervisor configuration register.
    HcrEl2,
    /// The counter-timer hypervisor control register.
    CnthctlEl2,
    /// The virtual counter offset.
    CntvoffEl2,
    /// EL2's saved program status register.
    SpsrEl2,
    /// EL2's exception link register.
    ElrEl2,
    /// The stack pointer of EL1.
    SpEl1,
    /// EL1's translation control register: the shape of the page tables.
    TcrEl1,
    /// EL1's memory attribute indirection register: the memory types page table
    /// entries can choose from.
    MairEl1,
    /// The base address of the page tables for the lower half of the address space.
    Ttbr0El1,
    /// The base address of the page tables for the upper half of the address space.
    Ttbr1El1,
}

impl SystemRegister {
    /// The register with this encoding, if Nebu models it.
    fn from_encoding(op0: u8, op1: u8, crn: u8, crm: u8, op2: u8) -> Option<Self> {
        Some(match (op0, op1, crn, crm, op2) {
            (3, 0, 0, 0, 5) => SystemRegister::MpidrEl1,
            (3, 0, 4, 2, 2) => SystemRegister::CurrentEl,
            (3, 0, 1, 0, 0) => SystemRegister::SctlrEl1,
            (3, 0, 4, 0, 0) => SystemRegister::SpsrEl1,
            (3, 0, 4, 0, 1) => SystemRegister::ElrEl1,
            (3, 0, 4, 1, 0) => SystemRegister::SpEl0,
            (3, 0, 5, 2, 0) => SystemRegister::EsrEl1,
            (3, 0, 6, 0, 0) => SystemRegister::FarEl1,
            (3, 0, 12, 0, 0) => SystemRegister::VbarEl1,
            (3, 4, 1, 1, 0) => SystemRegister::HcrEl2,
            (3, 4, 14, 1, 0) => SystemRegister::CnthctlEl2,
            (3, 4, 14, 0, 3) => SystemRegister::CntvoffEl2,
            (3, 4, 4, 0, 0) => SystemRegister::SpsrEl2,
            (3, 4, 4, 0, 1) => SystemRegister::ElrEl2,
            (3, 4, 4, 1, 0) => SystemRegister::SpEl1,
            (3, 0, 2, 0, 2) => SystemRegister::TcrEl1,
            (3, 0, 10, 2, 0) => SystemRegister::MairEl1,
            (3, 0, 2, 0, 0) => SystemRegister::Ttbr0El1,
            (3, 0, 2, 0, 1) => SystemRegister::Ttbr1El1,
            _ => return None,
        })
    }

    /// The register's architectural name.
    pub fn name(self) -> &'static str {
        match self {
            SystemRegister::MpidrEl1 => "MPIDR_EL1",
            SystemRegister::CurrentEl => "CurrentEL",
            SystemRegister::SctlrEl1 => "SCTLR_EL1",
            SystemRegister::SpsrEl1 => "SPSR_EL1",
            SystemRegister::ElrEl1 => "ELR_EL1",
            SystemRegister::SpEl0 => "SP_EL0",
            SystemRegister::EsrEl1 => "ESR_EL1",
            SystemRegister::FarEl1 => "FAR_EL1",
            SystemRegister::VbarEl1 => "VBAR_EL1",
            SystemRegister::HcrEl2 => "HCR_EL2",
            SystemRegister::CnthctlEl2 => "CNTHCTL_EL2",
            SystemRegister::CntvoffEl2 => "CNTVOFF_EL2",
            SystemRegister::SpsrEl2 => "SPSR_EL2",
            SystemRegister::ElrEl2 => "ELR_EL2",
            SystemRegister::SpEl1 => "SP_EL1",
            SystemRegister::TcrEl1 => "TCR_EL1",
            SystemRegister::MairEl1 => "MAIR_EL1",
            SystemRegister::Ttbr0El1 => "TTBR0_EL1",
            SystemRegister::Ttbr1El1 => "TTBR1_EL1",
        }
    }
}

/// A decoded instruction.
///
/// Register numbers are 0 to 31. Whether 31 means the zero register or the stack
/// pointer depends on the instruction; the comments say which.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Instruction {
    /// ADR: `rd = pc + offset`. `rd` 31 is the zero register.
    Adr { rd: u8, offset: i64 },
    /// ADRP: `rd = (pc with its low 12 bits cleared) + offset`, where `offset` is a
    /// multiple of 4096. `rd` 31 is the zero register.
    Adrp { rd: u8, offset: i64 },
    /// ADD (immediate): `rd = rn + imm`. 31 is SP for both registers.
    AddImmediate { width: Width, rd: u8, rn: u8, imm: u64 },
    /// SUB (immediate): `rd = rn - imm`. 31 is SP for both registers.
    SubImmediate { width: Width, rd: u8, rn: u8, imm: u64 },
    /// ADDS (immediate): `rd = rn + imm`, setting the flags. `rd` 31 is the zero
    /// register, `rn` 31 is SP.
    AddsImmediate { width: Width, rd: u8, rn: u8, imm: u64 },
    /// SUBS (immediate): `rd = rn - imm`, setting the flags. `rd` 31 is the zero
    /// register, `rn` 31 is SP.
    SubsImmediate { width: Width, rd: u8, rn: u8, imm: u64 },
    /// AND (immediate): `rd = rn & imm`. `rd` 31 is SP, `rn` 31 is the zero register.
    AndImmediate { width: Width, rd: u8, rn: u8, imm: u64 },
    /// ORR (immediate): `rd = rn | imm`. `rd` 31 is SP, `rn` 31 is the zero register.
    OrrImmediate { width: Width, rd: u8, rn: u8, imm: u64 },
    /// EOR (immediate): `rd = rn ^ imm`. `rd` 31 is SP, `rn` 31 is the zero register.
    EorImmediate { width: Width, rd: u8, rn: u8, imm: u64 },
    /// ANDS (immediate): `rd = rn & imm`, setting the flags. 31 is the zero register.
    AndsImmediate { width: Width, rd: u8, rn: u8, imm: u64 },
    /// MOVN: `rd = !(imm16 << shift)`. 31 is the zero register.
    Movn { width: Width, rd: u8, imm16: u16, shift: u8 },
    /// MOVZ: `rd = imm16 << shift`. 31 is the zero register.
    Movz { width: Width, rd: u8, imm16: u16, shift: u8 },
    /// MOVK: replace bits `shift..shift + 16` of `rd` with `imm16`. 31 is the zero register.
    Movk { width: Width, rd: u8, imm16: u16, shift: u8 },
    /// BFM: copy a bitfield from `rn` into `rd`, keeping `rd`'s other bits.
    /// 31 is the zero register.
    Bfm { width: Width, rd: u8, rn: u8, immr: u8, imms: u8 },
    /// SBFM: copy a bitfield from `rn` into `rd`, sign-extending it.
    /// 31 is the zero register.
    Sbfm { width: Width, rd: u8, rn: u8, immr: u8, imms: u8 },
    /// UBFM: copy a bitfield from `rn` into `rd`, zero-extending it.
    /// 31 is the zero register.
    Ubfm { width: Width, rd: u8, rn: u8, immr: u8, imms: u8 },
    /// ADD (shifted register): `rd = rn + (rm shifted)`. 31 is the zero register.
    AddShiftedRegister { width: Width, rd: u8, rn: u8, rm: u8, shift: Shift, amount: u8 },
    /// SUB (shifted register): `rd = rn - (rm shifted)`. 31 is the zero register.
    SubShiftedRegister { width: Width, rd: u8, rn: u8, rm: u8, shift: Shift, amount: u8 },
    /// SUBS (shifted register): `rd = rn - (rm shifted)`, setting the flags.
    /// 31 is the zero register.
    SubsShiftedRegister { width: Width, rd: u8, rn: u8, rm: u8, shift: Shift, amount: u8 },
    /// AND (shifted register): `rd = rn & (rm shifted)`. 31 is the zero register.
    AndShiftedRegister { width: Width, rd: u8, rn: u8, rm: u8, shift: Shift, amount: u8 },
    /// ORR (shifted register): `rd = rn | (rm shifted)`. 31 is the zero register.
    OrrShiftedRegister { width: Width, rd: u8, rn: u8, rm: u8, shift: Shift, amount: u8 },
    /// ORN (shifted register): `rd = rn | !(rm shifted)`. 31 is the zero register.
    OrnShiftedRegister { width: Width, rd: u8, rn: u8, rm: u8, shift: Shift, amount: u8 },
    /// EOR (shifted register): `rd = rn ^ (rm shifted)`. 31 is the zero register.
    EorShiftedRegister { width: Width, rd: u8, rn: u8, rm: u8, shift: Shift, amount: u8 },
    /// ANDS (shifted register): `rd = rn & (rm shifted)`, setting the flags.
    /// 31 is the zero register.
    AndsShiftedRegister { width: Width, rd: u8, rn: u8, rm: u8, shift: Shift, amount: u8 },
    /// LSLV and LSRV: `rd = rn` shifted by `rm` modulo the register width. `shift` is
    /// only ever `Lsl` or `Lsr`. 31 is the zero register.
    ShiftVariable { width: Width, rd: u8, rn: u8, rm: u8, shift: Shift },
    /// CSEL: `rd = if condition holds { rn } else { rm }`. 31 is the zero register.
    Csel { width: Width, rd: u8, rn: u8, rm: u8, condition: u8 },
    /// CSINC: `rd = if condition holds { rn } else { rm + 1 }`. 31 is the zero register.
    Csinc { width: Width, rd: u8, rn: u8, rm: u8, condition: u8 },
    /// CCMP (register): if `condition` holds, set the flags as `cmp rn, rm` would;
    /// otherwise set them to `nzcv`. 31 is the zero register.
    CcmpRegister { width: Width, rn: u8, rm: u8, nzcv: u8, condition: u8 },
    /// CCMP (immediate): if `condition` holds, set the flags as `cmp rn, #imm` would;
    /// otherwise set them to `nzcv`. 31 is the zero register.
    CcmpImmediate { width: Width, rn: u8, imm: u8, nzcv: u8, condition: u8 },
    /// B: branch to `pc + offset`.
    B { offset: i64 },
    /// BL: branch to `pc + offset`, putting the return address in x30.
    Bl { offset: i64 },
    /// BR: branch to the address in `rn`. 31 is the zero register.
    Br { rn: u8 },
    /// BLR: branch to the address in `rn`, putting the return address in x30. 31 is
    /// the zero register.
    Blr { rn: u8 },
    /// RET: branch to the address in `rn` (normally x30).
    Ret { rn: u8 },
    /// B.cond: branch to `pc + offset` if `condition` holds.
    BCond { condition: u8, offset: i64 },
    /// CBZ: branch to `pc + offset` if `rt` is zero. 31 is the zero register.
    Cbz { width: Width, rt: u8, offset: i64 },
    /// CBNZ: branch to `pc + offset` if `rt` is not zero. 31 is the zero register.
    Cbnz { width: Width, rt: u8, offset: i64 },
    /// TBZ: branch to `pc + offset` if bit `bit` of `rt` is zero.
    Tbz { rt: u8, bit: u8, offset: i64 },
    /// TBNZ: branch to `pc + offset` if bit `bit` of `rt` is one.
    Tbnz { rt: u8, bit: u8, offset: i64 },
    /// HLT: stop for an external debugger, or make a semihosting call.
    Hlt { imm16: u16 },
    /// BRK: take a Breakpoint Instruction exception.
    Brk { imm16: u16 },
    /// UDF: a permanently undefined instruction, which takes an Undefined Instruction
    /// exception.
    Udf { imm16: u16 },
    /// ERET: return from an exception, restoring PSTATE from the current SPSR and
    /// jumping to the current ELR.
    Eret,
    /// NOP: do nothing.
    Nop,
    /// WFE: wait for an event.
    Wfe,
    /// ISB: instruction synchronisation barrier.
    Isb,
    /// DSB: data synchronisation barrier. `option` is the CRm field, naming which
    /// accesses it waits for.
    Dsb { option: u8 },
    /// IC IALLU: invalidate the whole instruction cache.
    IcIallu,
    /// TLBI VMALLE1: invalidate every cached EL1&0 translation.
    TlbiVmalle1,
    /// MRS: read a system register into `rt`. 31 is the zero register.
    Mrs { rt: u8, register: SystemRegister },
    /// MSR (register): write `rt` to a system register. 31 is the zero register.
    Msr { register: SystemRegister, rt: u8 },
    /// LDR (literal): load `rt` from `pc + offset`. 31 is the zero register.
    LdrLiteral { width: Width, rt: u8, offset: i64 },
    /// LDR (immediate): load `rt` from memory. `rn` 31 is SP, `rt` 31 the zero register.
    LdrImmediate { width: Width, rt: u8, rn: u8, index: Index, offset: i64 },
    /// STR (immediate): store `rt` to memory. `rn` 31 is SP, `rt` 31 the zero register.
    StrImmediate { width: Width, rt: u8, rn: u8, index: Index, offset: i64 },
    /// LDRB (immediate): load a byte into `rt`, zero-extended.
    /// `rn` 31 is SP, `rt` 31 the zero register.
    LdrbImmediate { rt: u8, rn: u8, index: Index, offset: i64 },
    /// LDRB (register): load a byte from `rn + (rm extended)` into `rt`.
    /// `rn` 31 is SP; `rt` and `rm` 31 the zero register.
    LdrbRegister { rt: u8, rn: u8, rm: u8, extend: Extend },
    /// LDR (register): load `rt` from `rn + (rm extended, shifted by `amount`)`.
    /// `amount` is 0 or log2 of the access size. `rn` 31 is SP; `rt` and `rm` 31 the
    /// zero register.
    LdrRegister { width: Width, rt: u8, rn: u8, rm: u8, extend: Extend, amount: u8 },
    /// STR (register): store `rt` to `rn + (rm extended, shifted by `amount`)`.
    /// `amount` is 0 or log2 of the access size. `rn` 31 is SP; `rt` and `rm` 31 the
    /// zero register.
    StrRegister { width: Width, rt: u8, rn: u8, rm: u8, extend: Extend, amount: u8 },
    /// STRB (immediate): store the low 8 bits of `rt`.
    /// `rn` 31 is SP, `rt` 31 the zero register.
    StrbImmediate { rt: u8, rn: u8, index: Index, offset: i64 },
    /// STRH (immediate): store the low 16 bits of `rt`.
    /// `rn` 31 is SP, `rt` 31 the zero register.
    StrhImmediate { rt: u8, rn: u8, index: Index, offset: i64 },
    /// STUR: store `rt` to `rn + offset`, with an unscaled offset.
    /// `rn` 31 is SP, `rt` 31 the zero register.
    Stur { width: Width, rt: u8, rn: u8, offset: i64 },
    /// STP: store `rt` and `rt2` to consecutive locations.
    /// `rn` 31 is SP, `rt` and `rt2` 31 the zero register.
    Stp { width: Width, rt: u8, rt2: u8, rn: u8, index: Index, offset: i64 },
    /// LDP: load `rt` and `rt2` from consecutive locations.
    /// `rn` 31 is SP, `rt` and `rt2` 31 the zero register.
    Ldp { width: Width, rt: u8, rt2: u8, rn: u8, index: Index, offset: i64 },
}

/// Why a word could not be decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// The word is not an instruction Nebu implements.
    Unimplemented,
    /// The encoding is unallocated: real hardware raises an Undefined Instruction
    /// exception.
    Unallocated,
    /// An `MRS` or `MSR` of a system register Nebu does not model.
    SystemRegister { op0: u8, op1: u8, crn: u8, crm: u8, op2: u8 },
    /// The architecture leaves the behaviour CONSTRAINED UNPREDICTABLE.
    Unpredictable(&'static str),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Unimplemented => write!(f, "instruction not implemented"),
            DecodeError::Unallocated => write!(f, "unallocated encoding (undefined instruction)"),
            DecodeError::SystemRegister { op0, op1, crn, crm, op2 } => {
                write!(f, "system register S{op0}_{op1}_C{crn}_C{crm}_{op2} not implemented")
            }
            DecodeError::Unpredictable(why) => write!(f, "CONSTRAINED UNPREDICTABLE: {why}"),
        }
    }
}

/// Extracts bits `high..=low` of `word`.
fn bits(word: u32, high: u32, low: u32) -> u32 {
    (word >> low) & ((1 << (high - low + 1)) - 1)
}

/// Extracts a register number (a 5-bit field starting at `low`).
fn reg(word: u32, low: u32) -> u8 {
    bits(word, low + 4, low) as u8
}

/// Sign-extends the low `width` bits of `value`.
fn sign_extend(value: u32, width: u32) -> i64 {
    let shift = 64 - width;
    (i64::from(value) << shift) >> shift
}

/// Decodes one instruction word.
pub fn decode(word: u32) -> Result<Instruction, DecodeError> {
    match bits(word, 28, 25) {
        0b1000 | 0b1001 => data_processing_immediate(word),
        0b1010 | 0b1011 => branches_exceptions_system(word),
        op0 if op0 & 0b0101 == 0b0100 => loads_and_stores(word),
        op0 if op0 & 0b0111 == 0b0101 => data_processing_register(word),
        // Reserved: UDF is the only allocated encoding.
        0b0000 if bits(word, 31, 16) == 0 => {
            Ok(Instruction::Udf { imm16: bits(word, 15, 0) as u16 })
        }
        _ => Err(DecodeError::Unimplemented),
    }
}

fn data_processing_immediate(word: u32) -> Result<Instruction, DecodeError> {
    let width = Width::from_bit(bits(word, 31, 31));
    let rd = reg(word, 0);
    let rn = reg(word, 5);

    match bits(word, 25, 23) {
        // PC-relative addressing.
        0b000 | 0b001 => {
            let imm = (bits(word, 23, 5) << 2) | bits(word, 30, 29);
            let offset = sign_extend(imm, 21);
            Ok(if bits(word, 31, 31) == 1 {
                Instruction::Adrp { rd, offset: offset << 12 }
            } else {
                Instruction::Adr { rd, offset }
            })
        }
        // Add/subtract (immediate).
        0b010 => {
            let imm = u64::from(bits(word, 21, 10)) << (12 * bits(word, 22, 22));
            match bits(word, 30, 29) {
                0b00 => Ok(Instruction::AddImmediate { width, rd, rn, imm }),
                0b01 => Ok(Instruction::AddsImmediate { width, rd, rn, imm }),
                0b10 => Ok(Instruction::SubImmediate { width, rd, rn, imm }),
                _ => Ok(Instruction::SubsImmediate { width, rd, rn, imm }),
            }
        }
        // Logical (immediate).
        0b100 => {
            let n = bits(word, 22, 22);
            if width == Width::W32 && n == 1 {
                return Err(DecodeError::Unallocated);
            }
            let (imm, _) = decode_bit_masks(n, bits(word, 15, 10), bits(word, 21, 16), true, width)
                .ok_or(DecodeError::Unallocated)?;
            match bits(word, 30, 29) {
                0b00 => Ok(Instruction::AndImmediate { width, rd, rn, imm }),
                0b01 => Ok(Instruction::OrrImmediate { width, rd, rn, imm }),
                0b10 => Ok(Instruction::EorImmediate { width, rd, rn, imm }),
                _ => Ok(Instruction::AndsImmediate { width, rd, rn, imm }),
            }
        }
        // Move wide (immediate).
        0b101 => {
            let hw = bits(word, 22, 21);
            if width == Width::W32 && hw >= 2 {
                return Err(DecodeError::Unallocated);
            }
            let imm16 = bits(word, 20, 5) as u16;
            let shift = (hw * 16) as u8;
            match bits(word, 30, 29) {
                0b00 => Ok(Instruction::Movn { width, rd, imm16, shift }),
                0b10 => Ok(Instruction::Movz { width, rd, imm16, shift }),
                0b11 => Ok(Instruction::Movk { width, rd, imm16, shift }),
                _ => Err(DecodeError::Unallocated),
            }
        }
        // Bitfield.
        0b110 => {
            let n = bits(word, 22, 22);
            let immr = bits(word, 21, 16);
            let imms = bits(word, 15, 10);
            let wide = width == Width::W64;
            if (n == 1) != wide || (!wide && (immr >= 32 || imms >= 32)) {
                return Err(DecodeError::Unallocated);
            }
            let (immr, imms) = (immr as u8, imms as u8);
            match bits(word, 30, 29) {
                0b00 => Ok(Instruction::Sbfm { width, rd, rn, immr, imms }),
                0b01 => Ok(Instruction::Bfm { width, rd, rn, immr, imms }),
                0b10 => Ok(Instruction::Ubfm { width, rd, rn, immr, imms }),
                _ => Err(DecodeError::Unallocated),
            }
        }
        _ => Err(DecodeError::Unimplemented),
    }
}

/// The Arm ARM's `DecodeBitMasks`: expands the `N:imms:immr` fields of a logical or
/// bitfield instruction into a pair of masks `(wmask, tmask)`.
///
/// Returns `None` for reserved encodings.
pub fn decode_bit_masks(
    n: u32,
    imms: u32,
    immr: u32,
    immediate: bool,
    width: Width,
) -> Option<(u64, u64)> {
    // `len` is the position of the highest set bit of N:NOT(imms).
    let combined = (n << 6) | (!imms & 0x3F);
    if combined == 0 {
        return None;
    }
    let len = 31 - combined.leading_zeros();
    if len < 1 {
        return None;
    }
    let levels = (1 << len) - 1;
    if immediate && imms & levels == levels {
        return None;
    }

    let s = imms & levels;
    let r = immr & levels;
    let diff = s.wrapping_sub(r) & levels;
    let esize = 1u32 << len;

    let ones = |count: u32| {
        if count >= 64 { u64::MAX } else { (1u64 << count) - 1 }
    };
    let element_mask = ones(esize);
    let welem = ones(s + 1);
    let telem = ones(diff + 1);

    // Rotate `welem` right by `r` within an element of `esize` bits.
    let rotated =
        if r == 0 { welem } else { ((welem >> r) | (welem << (esize - r))) & element_mask };

    let replicate = |element: u64| {
        let mut value = 0u64;
        let mut position = 0;
        while position < width.bits() {
            value |= element << position;
            position += esize;
        }
        value & width.mask()
    };
    Some((replicate(rotated), replicate(telem)))
}

fn branches_exceptions_system(word: u32) -> Result<Instruction, DecodeError> {
    // Unconditional branch (immediate).
    if bits(word, 30, 26) == 0b00101 {
        let offset = sign_extend(bits(word, 25, 0), 26) * 4;
        return Ok(if bits(word, 31, 31) == 0 {
            Instruction::B { offset }
        } else {
            Instruction::Bl { offset }
        });
    }

    // Conditional branch (immediate).
    if bits(word, 31, 24) == 0b0101_0100 {
        if bits(word, 4, 4) == 1 {
            return Err(DecodeError::Unimplemented); // BC.cond
        }
        let offset = sign_extend(bits(word, 23, 5), 19) * 4;
        return Ok(Instruction::BCond { condition: bits(word, 3, 0) as u8, offset });
    }

    // Compare and branch (immediate).
    if bits(word, 30, 25) == 0b01_1010 {
        let width = Width::from_bit(bits(word, 31, 31));
        let offset = sign_extend(bits(word, 23, 5), 19) * 4;
        let rt = reg(word, 0);
        return Ok(if bits(word, 24, 24) == 0 {
            Instruction::Cbz { width, rt, offset }
        } else {
            Instruction::Cbnz { width, rt, offset }
        });
    }

    // Test and branch (immediate).
    if bits(word, 30, 25) == 0b01_1011 {
        let bit = ((bits(word, 31, 31) << 5) | bits(word, 23, 19)) as u8;
        let offset = sign_extend(bits(word, 18, 5), 14) * 4;
        let rt = reg(word, 0);
        return Ok(if bits(word, 24, 24) == 0 {
            Instruction::Tbz { rt, bit, offset }
        } else {
            Instruction::Tbnz { rt, bit, offset }
        });
    }

    // Unconditional branch (register).
    if bits(word, 31, 25) == 0b110_1011 {
        let (opc, op2, op3, op4) =
            (bits(word, 24, 21), bits(word, 20, 16), bits(word, 15, 10), bits(word, 4, 0));
        return match (opc, op2, op3, op4) {
            (0b0000, 0b11111, 0, 0) => Ok(Instruction::Br { rn: reg(word, 5) }),
            (0b0001, 0b11111, 0, 0) => Ok(Instruction::Blr { rn: reg(word, 5) }),
            (0b0010, 0b11111, 0, 0) => Ok(Instruction::Ret { rn: reg(word, 5) }),
            (0b0100, 0b11111, 0, 0) if reg(word, 5) == 31 => Ok(Instruction::Eret),
            _ => Err(DecodeError::Unimplemented),
        };
    }

    // Exception generation.
    if bits(word, 31, 24) == 0b1101_0100 {
        let (opc, op2, ll) = (bits(word, 23, 21), bits(word, 4, 2), bits(word, 1, 0));
        let imm16 = bits(word, 20, 5) as u16;
        match (opc, op2, ll) {
            (0b010, 0b000, 0b00) => return Ok(Instruction::Hlt { imm16 }),
            (0b001, 0b000, 0b00) => return Ok(Instruction::Brk { imm16 }),
            _ => {}
        }
        return Err(DecodeError::Unimplemented);
    }

    // System instructions.
    if bits(word, 31, 22) == 0b11_0101_0100 {
        return system(word);
    }

    Err(DecodeError::Unimplemented)
}

fn system(word: u32) -> Result<Instruction, DecodeError> {
    let l = bits(word, 21, 21);
    let op0 = bits(word, 20, 19) as u8;
    let op1 = bits(word, 18, 16) as u8;
    let crn = bits(word, 15, 12) as u8;
    let crm = bits(word, 11, 8) as u8;
    let op2 = bits(word, 7, 5) as u8;
    let rt = reg(word, 0);

    match (l, op0, op1, crn) {
        // Hints.
        (0, 0, 0b011, 0b0010) if rt == 31 => match (crm, op2) {
            (0, 0) => Ok(Instruction::Nop),
            (0, 2) => Ok(Instruction::Wfe),
            _ => Err(DecodeError::Unimplemented),
        },
        // Barriers.
        (0, 0, 0b011, 0b0011) if rt == 31 => match (op2, crm) {
            // CRm 0b0000 and 0b0100 with DSB's op2 are SSBB and PSSBB instead.
            (0b100, 0b0000 | 0b0100) => Err(DecodeError::Unimplemented),
            (0b100, option) => Ok(Instruction::Dsb { option }),
            (0b110, _) => Ok(Instruction::Isb),
            _ => Err(DecodeError::Unimplemented),
        },
        // SYS: cache and TLB maintenance and address translation.
        (0, 1, _, _) => match (op1, crn, crm, op2, rt) {
            (0, 7, 5, 0, 31) => Ok(Instruction::IcIallu),
            (0, 8, 7, 0, 31) => Ok(Instruction::TlbiVmalle1),
            _ => Err(DecodeError::Unimplemented),
        },
        // MRS and MSR (register).
        (_, 2 | 3, _, _) => {
            let register = SystemRegister::from_encoding(op0, op1, crn, crm, op2)
                .ok_or(DecodeError::SystemRegister { op0, op1, crn, crm, op2 })?;
            Ok(if l == 1 {
                Instruction::Mrs { rt, register }
            } else {
                Instruction::Msr { register, rt }
            })
        }
        _ => Err(DecodeError::Unimplemented),
    }
}

fn loads_and_stores(word: u32) -> Result<Instruction, DecodeError> {
    if bits(word, 26, 26) == 1 {
        return Err(DecodeError::Unimplemented); // SIMD and floating point
    }
    let rt = reg(word, 0);
    let rn = reg(word, 5);

    // Load register (literal).
    if bits(word, 29, 27) == 0b011 && bits(word, 25, 24) == 0b00 {
        let offset = sign_extend(bits(word, 23, 5), 19) * 4;
        return match bits(word, 31, 30) {
            0b00 => Ok(Instruction::LdrLiteral { width: Width::W32, rt, offset }),
            0b01 => Ok(Instruction::LdrLiteral { width: Width::W64, rt, offset }),
            _ => Err(DecodeError::Unimplemented), // LDRSW, PRFM
        };
    }

    // Load/store register pair.
    if bits(word, 29, 27) == 0b101 {
        let index = match bits(word, 24, 23) {
            0b01 => Index::Post,
            0b10 => Index::Offset,
            0b11 => Index::Pre,
            _ => return Err(DecodeError::Unimplemented), // STNP, LDNP
        };
        let width = match bits(word, 31, 30) {
            0b00 => Width::W32,
            0b10 => Width::W64,
            _ => return Err(DecodeError::Unimplemented), // LDPSW, STGP
        };
        let load = bits(word, 22, 22) == 1;
        let rt2 = reg(word, 10);
        if index != Index::Offset && rn != 31 && (rt == rn || rt2 == rn) {
            return Err(DecodeError::Unpredictable("writeback to a transfer register"));
        }
        if load && rt == rt2 {
            return Err(DecodeError::Unpredictable("LDP loads the same register twice"));
        }
        let offset = sign_extend(bits(word, 21, 15), 7) * width.bytes() as i64;
        return Ok(if load {
            Instruction::Ldp { width, rt, rt2, rn, index, offset }
        } else {
            Instruction::Stp { width, rt, rt2, rn, index, offset }
        });
    }

    // Load/store register (single register).
    if bits(word, 29, 27) == 0b111 {
        let size = bits(word, 31, 30);
        let opc = bits(word, 23, 22);
        let scale = 1i64 << size;

        let (index, offset) = match (bits(word, 25, 24), bits(word, 21, 21), bits(word, 11, 10)) {
            // Unsigned offset, scaled by the access size.
            (0b01, _, _) => (Index::Offset, i64::from(bits(word, 21, 10)) * scale),
            // Unscaled immediate offset.
            (0b00, 0, 0b00) => {
                return match (size, opc) {
                    (0b10 | 0b11, 0b00) => {
                        let width = Width::from_bit(size & 1);
                        let offset = sign_extend(bits(word, 20, 12), 9);
                        Ok(Instruction::Stur { width, rt, rn, offset })
                    }
                    _ => Err(DecodeError::Unimplemented), // LDUR and narrower STUR forms
                };
            }
            (0b00, 0, 0b01) => (Index::Post, sign_extend(bits(word, 20, 12), 9)),
            (0b00, 0, 0b11) => (Index::Pre, sign_extend(bits(word, 20, 12), 9)),
            // Register offset.
            (0b00, 1, 0b10) => {
                let extend = match bits(word, 15, 13) {
                    0b010 => Extend::Uxtw,
                    0b011 => Extend::Lsl,
                    0b110 => Extend::Sxtw,
                    0b111 => Extend::Sxtx,
                    _ => return Err(DecodeError::Unallocated),
                };
                return match (size, opc) {
                    // For a byte access the shift amount is always zero, whatever S says.
                    (0b00, 0b01) => {
                        Ok(Instruction::LdrbRegister { rt, rn, rm: reg(word, 16), extend })
                    }
                    (0b10 | 0b11, 0b00 | 0b01) => {
                        let width = Width::from_bit(size & 1);
                        let amount = if bits(word, 12, 12) == 1 { size as u8 } else { 0 };
                        let rm = reg(word, 16);
                        Ok(if opc == 0b01 {
                            Instruction::LdrRegister { width, rt, rn, rm, extend, amount }
                        } else {
                            Instruction::StrRegister { width, rt, rn, rm, extend, amount }
                        })
                    }
                    _ => Err(DecodeError::Unimplemented),
                };
            }
            _ => return Err(DecodeError::Unimplemented),
        };

        if index != Index::Offset && rn != 31 && rt == rn {
            return Err(DecodeError::Unpredictable("writeback to the transfer register"));
        }
        return match (size, opc) {
            (0b00, 0b00) => Ok(Instruction::StrbImmediate { rt, rn, index, offset }),
            (0b00, 0b01) => Ok(Instruction::LdrbImmediate { rt, rn, index, offset }),
            (0b01, 0b00) => Ok(Instruction::StrhImmediate { rt, rn, index, offset }),
            (0b10 | 0b11, 0b00) => {
                let width = Width::from_bit(size & 1);
                Ok(Instruction::StrImmediate { width, rt, rn, index, offset })
            }
            (0b10 | 0b11, 0b01) => {
                let width = Width::from_bit(size & 1);
                Ok(Instruction::LdrImmediate { width, rt, rn, index, offset })
            }
            _ => Err(DecodeError::Unimplemented),
        };
    }

    Err(DecodeError::Unimplemented)
}

fn data_processing_register(word: u32) -> Result<Instruction, DecodeError> {
    // Logical (shifted register).
    if bits(word, 28, 24) == 0b01010 {
        let width = Width::from_bit(bits(word, 31, 31));
        let amount = bits(word, 15, 10);
        if width == Width::W32 && amount >= 32 {
            return Err(DecodeError::Unallocated);
        }
        let shift = match bits(word, 23, 22) {
            0b00 => Shift::Lsl,
            0b01 => Shift::Lsr,
            0b10 => Shift::Asr,
            _ => Shift::Ror,
        };
        let (rd, rn, rm, amount) = (reg(word, 0), reg(word, 5), reg(word, 16), amount as u8);
        return match (bits(word, 30, 29), bits(word, 21, 21)) {
            (0b00, 0) => Ok(Instruction::AndShiftedRegister { width, rd, rn, rm, shift, amount }),
            (0b01, 0) => Ok(Instruction::OrrShiftedRegister { width, rd, rn, rm, shift, amount }),
            (0b01, 1) => Ok(Instruction::OrnShiftedRegister { width, rd, rn, rm, shift, amount }),
            (0b10, 0) => Ok(Instruction::EorShiftedRegister { width, rd, rn, rm, shift, amount }),
            (0b11, 0) => Ok(Instruction::AndsShiftedRegister { width, rd, rn, rm, shift, amount }),
            _ => Err(DecodeError::Unimplemented), // BIC, EON, BICS
        };
    }

    // Add/subtract (shifted register).
    if bits(word, 28, 24) == 0b01011 && bits(word, 21, 21) == 0 {
        let width = Width::from_bit(bits(word, 31, 31));
        let amount = bits(word, 15, 10);
        let shift = match bits(word, 23, 22) {
            0b00 => Shift::Lsl,
            0b01 => Shift::Lsr,
            0b10 => Shift::Asr,
            _ => return Err(DecodeError::Unallocated),
        };
        if width == Width::W32 && amount >= 32 {
            return Err(DecodeError::Unallocated);
        }
        let (rd, rn, rm, amount) = (reg(word, 0), reg(word, 5), reg(word, 16), amount as u8);
        return match bits(word, 30, 29) {
            0b00 => Ok(Instruction::AddShiftedRegister { width, rd, rn, rm, shift, amount }),
            0b10 => Ok(Instruction::SubShiftedRegister { width, rd, rn, rm, shift, amount }),
            0b11 => Ok(Instruction::SubsShiftedRegister { width, rd, rn, rm, shift, amount }),
            _ => Err(DecodeError::Unimplemented), // ADDS
        };
    }

    // Data-processing (2 source).
    if bits(word, 30, 21) == 0b0_1101_0110 {
        let width = Width::from_bit(bits(word, 31, 31));
        let (rd, rn, rm) = (reg(word, 0), reg(word, 5), reg(word, 16));
        return match bits(word, 15, 10) {
            0b00_1000 => Ok(Instruction::ShiftVariable { width, rd, rn, rm, shift: Shift::Lsl }),
            0b00_1001 => Ok(Instruction::ShiftVariable { width, rd, rn, rm, shift: Shift::Lsr }),
            _ => Err(DecodeError::Unimplemented), // ASRV, RORV, division, CRC32 and others
        };
    }

    // Conditional select.
    if bits(word, 28, 21) == 0b1101_0100 {
        if bits(word, 29, 29) == 1 {
            return Err(DecodeError::Unallocated);
        }
        let width = Width::from_bit(bits(word, 31, 31));
        let (rd, rn, rm) = (reg(word, 0), reg(word, 5), reg(word, 16));
        let condition = bits(word, 15, 12) as u8;
        return match (bits(word, 30, 30), bits(word, 11, 10)) {
            (0, 0b00) => Ok(Instruction::Csel { width, rd, rn, rm, condition }),
            (0, 0b01) => Ok(Instruction::Csinc { width, rd, rn, rm, condition }),
            _ => Err(DecodeError::Unimplemented), // CSINV, CSNEG
        };
    }

    // Conditional compare (register and immediate).
    if bits(word, 28, 21) == 0b1101_0010 {
        if bits(word, 29, 29) == 0 || bits(word, 10, 10) == 1 || bits(word, 4, 4) == 1 {
            return Err(DecodeError::Unallocated);
        }
        let width = Width::from_bit(bits(word, 31, 31));
        let rn = reg(word, 5);
        // Bits 20 to 16 are either rm or the immediate.
        let (rm, imm) = (reg(word, 16), reg(word, 16));
        let (nzcv, condition) = (bits(word, 3, 0) as u8, bits(word, 15, 12) as u8);
        return match (bits(word, 30, 30), bits(word, 11, 11)) {
            (1, 0) => Ok(Instruction::CcmpRegister { width, rn, rm, nzcv, condition }),
            (1, _) => Ok(Instruction::CcmpImmediate { width, rn, imm, nzcv, condition }),
            _ => Err(DecodeError::Unimplemented), // CCMN
        };
    }
    Err(DecodeError::Unimplemented)
}

/// Formats register `n` of `width`, where 31 is either SP or the zero register.
struct RegName {
    n: u8,
    width: Width,
    sp: bool,
}

impl fmt::Display for RegName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.n, self.width, self.sp) {
            (31, Width::W64, true) => write!(f, "sp"),
            (31, Width::W32, true) => write!(f, "wsp"),
            (31, Width::W64, false) => write!(f, "xzr"),
            (31, Width::W32, false) => write!(f, "wzr"),
            (n, Width::W64, _) => write!(f, "x{n}"),
            (n, Width::W32, _) => write!(f, "w{n}"),
        }
    }
}

fn zr(n: u8, width: Width) -> RegName {
    RegName { n, width, sp: false }
}

fn sp(n: u8, width: Width) -> RegName {
    RegName { n, width, sp: true }
}

/// Formats a PC-relative offset.
struct Relative(i64);

impl fmt::Display for Relative {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 < 0 {
            write!(f, "#-{:#x}", self.0.unsigned_abs())
        } else {
            write!(f, "#+{:#x}", self.0)
        }
    }
}

/// Formats a signed immediate.
struct Signed(i64);

impl fmt::Display for Signed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 < 0 {
            write!(f, "#-{:#x}", self.0.unsigned_abs())
        } else {
            write!(f, "#{:#x}", self.0)
        }
    }
}

/// Formats a memory operand: `[base, #offset]`, `[base, #offset]!` or `[base], #offset`.
struct Address {
    rn: u8,
    index: Index,
    offset: i64,
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let base = sp(self.rn, Width::W64);
        match self.index {
            Index::Offset if self.offset == 0 => write!(f, "[{base}]"),
            Index::Offset => write!(f, "[{base}, {}]", Signed(self.offset)),
            Index::Pre => write!(f, "[{base}, {}]!", Signed(self.offset)),
            Index::Post => write!(f, "[{base}], {}", Signed(self.offset)),
        }
    }
}

fn shift_name(shift: Shift) -> &'static str {
    match shift {
        Shift::Lsl => "lsl",
        Shift::Lsr => "lsr",
        Shift::Asr => "asr",
        Shift::Ror => "ror",
    }
}

/// Formats a three-register instruction whose last register is shifted.
struct ShiftedRegister {
    name: &'static str,
    width: Width,
    rd: u8,
    rn: u8,
    rm: u8,
    shift: Shift,
    amount: u8,
}

impl fmt::Display for ShiftedRegister {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { name, width, rd, rn, rm, shift, amount } = *self;
        write!(
            f,
            "{name} {}, {}, {}, {} #{amount}",
            zr(rd, width),
            zr(rn, width),
            zr(rm, width),
            shift_name(shift)
        )
    }
}

/// The assembler names of DSB's options, indexed by CRm. Empty entries have no name.
const BARRIER_OPTIONS: [&str; 16] = [
    "", "oshld", "oshst", "osh", "", "nshld", "nshst", "nsh", "", "ishld", "ishst", "ish", "",
    "ld", "st", "sy",
];

/// The assembler name of a register offset's extension, and the width of the offset
/// register it applies to.
fn extend_name(extend: Extend) -> (&'static str, Width) {
    match extend {
        Extend::Uxtw => ("uxtw", Width::W32),
        Extend::Lsl => ("lsl", Width::W64),
        Extend::Sxtw => ("sxtw", Width::W32),
        Extend::Sxtx => ("sxtx", Width::W64),
    }
}

/// The assembler names of the condition codes.
const CONDITIONS: [&str; 16] = [
    "eq", "ne", "hs", "lo", "mi", "pl", "vs", "vc", "hi", "ls", "ge", "lt", "gt", "le", "al", "nv",
];

impl fmt::Display for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use Instruction::*;
        match *self {
            Adr { rd, offset } => write!(f, "adr {}, {}", zr(rd, Width::W64), Relative(offset)),
            Adrp { rd, offset } => write!(f, "adrp {}, {}", zr(rd, Width::W64), Relative(offset)),
            AddImmediate { width, rd, rn, imm } => {
                write!(f, "add {}, {}, #{imm:#x}", sp(rd, width), sp(rn, width))
            }
            SubImmediate { width, rd, rn, imm } => {
                write!(f, "sub {}, {}, #{imm:#x}", sp(rd, width), sp(rn, width))
            }
            AndImmediate { width, rd, rn, imm } => {
                write!(f, "and {}, {}, #{imm:#x}", sp(rd, width), zr(rn, width))
            }
            OrrImmediate { width, rd, rn, imm } => {
                write!(f, "orr {}, {}, #{imm:#x}", sp(rd, width), zr(rn, width))
            }
            EorImmediate { width, rd, rn, imm } => {
                write!(f, "eor {}, {}, #{imm:#x}", sp(rd, width), zr(rn, width))
            }
            AndsImmediate { width, rd, rn, imm } => {
                write!(f, "ands {}, {}, #{imm:#x}", zr(rd, width), zr(rn, width))
            }
            Movn { width, rd, imm16, shift } => {
                write!(f, "movn {}, #{imm16:#x}, lsl #{shift}", zr(rd, width))
            }
            Movz { width, rd, imm16, shift } => {
                write!(f, "movz {}, #{imm16:#x}, lsl #{shift}", zr(rd, width))
            }
            Movk { width, rd, imm16, shift } => {
                write!(f, "movk {}, #{imm16:#x}, lsl #{shift}", zr(rd, width))
            }
            Bfm { width, rd, rn, immr, imms } => {
                write!(f, "bfm {}, {}, #{immr}, #{imms}", zr(rd, width), zr(rn, width))
            }
            Sbfm { width, rd, rn, immr, imms } => {
                write!(f, "sbfm {}, {}, #{immr}, #{imms}", zr(rd, width), zr(rn, width))
            }
            Ubfm { width, rd, rn, immr, imms } => {
                write!(f, "ubfm {}, {}, #{immr}, #{imms}", zr(rd, width), zr(rn, width))
            }
            AddsImmediate { width, rd, rn, imm } => {
                write!(f, "adds {}, {}, #{imm:#x}", zr(rd, width), sp(rn, width))
            }
            SubsImmediate { width, rd, rn, imm } => {
                write!(f, "subs {}, {}, #{imm:#x}", zr(rd, width), sp(rn, width))
            }
            AddShiftedRegister { width, rd, rn, rm, shift, amount } => write!(
                f,
                "add {}, {}, {}, {} #{amount}",
                zr(rd, width),
                zr(rn, width),
                zr(rm, width),
                shift_name(shift)
            ),
            SubShiftedRegister { width, rd, rn, rm, shift, amount } => {
                ShiftedRegister { name: "sub", width, rd, rn, rm, shift, amount }.fmt(f)
            }
            SubsShiftedRegister { width, rd, rn, rm, shift, amount } => write!(
                f,
                "subs {}, {}, {}, {} #{amount}",
                zr(rd, width),
                zr(rn, width),
                zr(rm, width),
                shift_name(shift)
            ),
            OrnShiftedRegister { width, rd, rn, rm, shift, amount } => {
                ShiftedRegister { name: "orn", width, rd, rn, rm, shift, amount }.fmt(f)
            }
            AndShiftedRegister { width, rd, rn, rm, shift, amount } => {
                ShiftedRegister { name: "and", width, rd, rn, rm, shift, amount }.fmt(f)
            }
            OrrShiftedRegister { width, rd, rn, rm, shift, amount } => {
                ShiftedRegister { name: "orr", width, rd, rn, rm, shift, amount }.fmt(f)
            }
            EorShiftedRegister { width, rd, rn, rm, shift, amount } => {
                ShiftedRegister { name: "eor", width, rd, rn, rm, shift, amount }.fmt(f)
            }
            AndsShiftedRegister { width, rd, rn, rm, shift, amount } => {
                ShiftedRegister { name: "ands", width, rd, rn, rm, shift, amount }.fmt(f)
            }
            ShiftVariable { width, rd, rn, rm, shift } => write!(
                f,
                "{}v {}, {}, {}",
                shift_name(shift),
                zr(rd, width),
                zr(rn, width),
                zr(rm, width)
            ),
            Csel { width, rd, rn, rm, condition } => write!(
                f,
                "csel {}, {}, {}, {}",
                zr(rd, width),
                zr(rn, width),
                zr(rm, width),
                CONDITIONS[usize::from(condition)]
            ),
            Csinc { width, rd, rn, rm, condition } => write!(
                f,
                "csinc {}, {}, {}, {}",
                zr(rd, width),
                zr(rn, width),
                zr(rm, width),
                CONDITIONS[usize::from(condition)]
            ),
            CcmpRegister { width, rn, rm, nzcv, condition } => write!(
                f,
                "ccmp {}, {}, #{nzcv:#x}, {}",
                zr(rn, width),
                zr(rm, width),
                CONDITIONS[usize::from(condition)]
            ),
            CcmpImmediate { width, rn, imm, nzcv, condition } => write!(
                f,
                "ccmp {}, #{imm:#x}, #{nzcv:#x}, {}",
                zr(rn, width),
                CONDITIONS[usize::from(condition)]
            ),
            B { offset } => write!(f, "b {}", Relative(offset)),
            Bl { offset } => write!(f, "bl {}", Relative(offset)),
            Br { rn } => write!(f, "br {}", zr(rn, Width::W64)),
            Blr { rn } => write!(f, "blr {}", zr(rn, Width::W64)),
            Ret { rn } => write!(f, "ret {}", zr(rn, Width::W64)),
            BCond { condition, offset } => {
                write!(f, "b.{} {}", CONDITIONS[usize::from(condition)], Relative(offset))
            }
            Cbz { width, rt, offset } => write!(f, "cbz {}, {}", zr(rt, width), Relative(offset)),
            Cbnz { width, rt, offset } => {
                write!(f, "cbnz {}, {}", zr(rt, width), Relative(offset))
            }
            Tbz { rt, bit, offset } => {
                let width = if bit >= 32 { Width::W64 } else { Width::W32 };
                write!(f, "tbz {}, #{bit}, {}", zr(rt, width), Relative(offset))
            }
            Tbnz { rt, bit, offset } => {
                let width = if bit >= 32 { Width::W64 } else { Width::W32 };
                write!(f, "tbnz {}, #{bit}, {}", zr(rt, width), Relative(offset))
            }
            Hlt { imm16 } => write!(f, "hlt #{imm16:#x}"),
            Brk { imm16 } => write!(f, "brk #{imm16:#x}"),
            Udf { imm16 } => write!(f, "udf #{imm16:#x}"),
            Eret => write!(f, "eret"),
            Nop => write!(f, "nop"),
            Wfe => write!(f, "wfe"),
            Isb => write!(f, "isb"),
            Dsb { option } => match BARRIER_OPTIONS[usize::from(option)] {
                "" => write!(f, "dsb #{option}"),
                name => write!(f, "dsb {name}"),
            },
            IcIallu => write!(f, "ic iallu"),
            TlbiVmalle1 => write!(f, "tlbi vmalle1"),
            Mrs { rt, register } => write!(f, "mrs {}, {}", zr(rt, Width::W64), register.name()),
            Msr { register, rt } => write!(f, "msr {}, {}", register.name(), zr(rt, Width::W64)),
            LdrLiteral { width, rt, offset } => {
                write!(f, "ldr {}, {}", zr(rt, width), Relative(offset))
            }
            LdrImmediate { width, rt, rn, index, offset } => {
                write!(f, "ldr {}, {}", zr(rt, width), Address { rn, index, offset })
            }
            StrImmediate { width, rt, rn, index, offset } => {
                write!(f, "str {}, {}", zr(rt, width), Address { rn, index, offset })
            }
            LdrbImmediate { rt, rn, index, offset } => {
                write!(f, "ldrb {}, {}", zr(rt, Width::W32), Address { rn, index, offset })
            }
            LdrRegister { width, rt, rn, rm, extend, amount } => {
                let (name, rm_width) = extend_name(extend);
                write!(
                    f,
                    "ldr {}, [{}, {}, {name} #{amount}]",
                    zr(rt, width),
                    sp(rn, Width::W64),
                    zr(rm, rm_width)
                )
            }
            StrRegister { width, rt, rn, rm, extend, amount } => {
                let (name, rm_width) = extend_name(extend);
                write!(
                    f,
                    "str {}, [{}, {}, {name} #{amount}]",
                    zr(rt, width),
                    sp(rn, Width::W64),
                    zr(rm, rm_width)
                )
            }
            LdrbRegister { rt, rn, rm, extend } => {
                let (name, width) = match extend {
                    Extend::Uxtw => ("uxtw", Width::W32),
                    Extend::Lsl => ("lsl", Width::W64),
                    Extend::Sxtw => ("sxtw", Width::W32),
                    Extend::Sxtx => ("sxtx", Width::W64),
                };
                write!(
                    f,
                    "ldrb {}, [{}, {}, {name} #0]",
                    zr(rt, Width::W32),
                    sp(rn, Width::W64),
                    zr(rm, width)
                )
            }
            StrbImmediate { rt, rn, index, offset } => {
                write!(f, "strb {}, {}", zr(rt, Width::W32), Address { rn, index, offset })
            }
            StrhImmediate { rt, rn, index, offset } => {
                write!(f, "strh {}, {}", zr(rt, Width::W32), Address { rn, index, offset })
            }
            Stur { width, rt, rn, offset } => write!(
                f,
                "stur {}, {}",
                zr(rt, width),
                Address { rn, index: Index::Offset, offset }
            ),
            Stp { width, rt, rt2, rn, index, offset } => write!(
                f,
                "stp {}, {}, {}",
                zr(rt, width),
                zr(rt2, width),
                Address { rn, index, offset }
            ),
            Ldp { width, rt, rt2, rn, index, offset } => write!(
                f,
                "ldp {}, {}, {}",
                zr(rt, width),
                zr(rt2, width),
                Address { rn, index, offset }
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Instruction::*;
    use Width::{W32, W64};

    /// Encodings taken from the NeOS step 2 kernel, checked against llvm-objdump.
    #[test]
    fn decodes_the_instructions_neos_uses() {
        let cases = [
            (0xd53800a1, Mrs { rt: 1, register: SystemRegister::MpidrEl1 }),
            (0xd538424a, Mrs { rt: 10, register: SystemRegister::CurrentEl }),
            (0x92401c21, AndImmediate { width: W64, rd: 1, rn: 1, imm: 0xff }),
            (0xb4000061, Cbz { width: W64, rt: 1, offset: 0xc }),
            (0xd503205f, Wfe),
            (0xd503201f, Nop),
            (0xd5033fdf, Isb),
            (0x17ffffff, B { offset: -4 }),
            (0x94000100, Bl { offset: 0x400 }),
            (0x54000062, BCond { condition: 2, offset: 0xc }),
            (0x100821c1, Adr { rd: 1, offset: 0x10438 }),
            (0x9100003f, AddImmediate { width: W64, rd: 31, rn: 1, imm: 0 }),
            (0x910003e1, AddImmediate { width: W64, rd: 1, rn: 31, imm: 0 }),
            (0xd10043ff, SubImmediate { width: W64, rd: 31, rn: 31, imm: 0x10 }),
            (
                0xeb02003f,
                SubsShiftedRegister {
                    width: W64,
                    rd: 31,
                    rn: 1,
                    rm: 2,
                    shift: Shift::Lsl,
                    amount: 0,
                },
            ),
            (0x52820308, Movz { width: W32, rd: 8, imm16: 0x1018, shift: 0 }),
            (0x72a7e408, Movk { width: W32, rd: 8, imm16: 0x3f20, shift: 16 }),
            (0x33020d49, Bfm { width: W32, rd: 9, rn: 10, immr: 2, imms: 3 }),
            (0x36280089, Tbz { rt: 9, bit: 5, offset: 0x10 }),
            (0x372fffc9, Tbnz { rt: 9, bit: 5, offset: -8 }),
            (0xd45e0000, Hlt { imm16: 0xf000 }),
            (
                0xb900191f,
                StrImmediate { width: W32, rt: 31, rn: 8, index: Index::Offset, offset: 0x18 },
            ),
            (0xf800843f, StrImmediate { width: W64, rt: 31, rn: 1, index: Index::Post, offset: 8 }),
            (
                0xf81f0ffe,
                StrImmediate { width: W64, rt: 30, rn: 31, index: Index::Pre, offset: -16 },
            ),
            (
                0xb9400109,
                LdrImmediate { width: W32, rt: 9, rn: 8, index: Index::Offset, offset: 0 },
            ),
            (0xb81e8109, Stur { width: W32, rt: 9, rn: 8, offset: -0x18 }),
            (
                0xa9007fea,
                Stp { width: W64, rt: 10, rt2: 31, rn: 31, index: Index::Offset, offset: 0 },
            ),
            // The panic path.
            (0xd65f03c0, Ret { rn: 30 }),
            (0xf100391f, SubsImmediate { width: W64, rd: 31, rn: 8, imm: 0xe }),
            (0x7100297f, SubsImmediate { width: W32, rd: 31, rn: 11, imm: 0xa }),
            (
                0x8b010009,
                AddShiftedRegister {
                    width: W64,
                    rd: 9,
                    rn: 0,
                    rm: 1,
                    shift: Shift::Lsl,
                    amount: 0,
                },
            ),
            (
                0xaa1f03e8,
                OrrShiftedRegister {
                    width: W64,
                    rd: 8,
                    rn: 31,
                    rm: 31,
                    shift: Shift::Lsl,
                    amount: 0,
                },
            ),
            (0x3868694c, LdrbRegister { rt: 12, rn: 10, rm: 8, extend: Extend::Lsl }),
            (0x3840140b, LdrbImmediate { rt: 11, rn: 0, index: Index::Post, offset: 1 }),
            (0x790053e8, StrhImmediate { rt: 8, rn: 31, index: Index::Offset, offset: 0x28 }),
            (
                0xf9400009,
                LdrImmediate { width: W64, rt: 9, rn: 0, index: Index::Offset, offset: 0 },
            ),
            (
                0xa9bf23fe,
                Stp { width: W64, rt: 30, rt2: 8, rn: 31, index: Index::Pre, offset: -16 },
            ),
            // Step 3: the chainloader, and the boot code that relocates it.
            (0x58000382, LdrLiteral { width: W64, rt: 2, offset: 0x70 }),
            (0xd61f0020, Br { rn: 1 }),
            (0xd5033f9f, Dsb { option: 0b1111 }),
            (0xd508751f, IcIallu),
            (0x3100043f, AddsImmediate { width: W32, rd: 31, rn: 1, imm: 1 }),
            (0x1280001b, Movn { width: W32, rd: 27, imm16: 0, shift: 0 }),
            (0x53040f61, Ubfm { width: W32, rd: 1, rn: 27, immr: 4, imms: 3 }),
            (0x1300037d, Sbfm { width: W32, rd: 29, rn: 27, immr: 0, imms: 0 }),
            (
                0x0a817ee1,
                AndShiftedRegister {
                    width: W32,
                    rd: 1,
                    rn: 23,
                    rm: 1,
                    shift: Shift::Asr,
                    amount: 31,
                },
            ),
            (
                0x4a5d1821,
                EorShiftedRegister {
                    width: W32,
                    rd: 1,
                    rn: 1,
                    rm: 29,
                    shift: Shift::Lsr,
                    amount: 6,
                },
            ),
            (
                0xa94023e0,
                Ldp { width: W64, rt: 0, rt2: 8, rn: 31, index: Index::Offset, offset: 0 },
            ),
            (0x390002a4, StrbImmediate { rt: 4, rn: 21, index: Index::Offset, offset: 0 }),
            // Stage 1: dropping to EL1, the exception vectors and the fault reports.
            (0xd51c1101, Msr { register: SystemRegister::HcrEl2, rt: 1 }),
            (0xd53ce101, Mrs { rt: 1, register: SystemRegister::CnthctlEl2 }),
            (0xd51ce07f, Msr { register: SystemRegister::CntvoffEl2, rt: 31 }),
            (0xd5181001, Msr { register: SystemRegister::SctlrEl1, rt: 1 }),
            (0xd51c4001, Msr { register: SystemRegister::SpsrEl2, rt: 1 }),
            (0xd51c4021, Msr { register: SystemRegister::ElrEl2, rt: 1 }),
            (0xd518c008, Msr { register: SystemRegister::VbarEl1, rt: 8 }),
            (0xd5384102, Mrs { rt: 2, register: SystemRegister::SpEl0 }),
            (0xd5384022, Mrs { rt: 2, register: SystemRegister::ElrEl1 }),
            (0xd5384003, Mrs { rt: 3, register: SystemRegister::SpsrEl1 }),
            (0xd5385202, Mrs { rt: 2, register: SystemRegister::EsrEl1 }),
            (0xd5386003, Mrs { rt: 3, register: SystemRegister::FarEl1 }),
            (0xd69f03e0, Eret),
            (0xd4200000, Brk { imm16: 0 }),
            (0x00000000, Udf { imm16: 0 }),
            (0xb2400421, OrrImmediate { width: W64, rd: 1, rn: 1, imm: 0b11 }),
            (0xb5001f49, Cbnz { width: W64, rt: 9, offset: 0x3e8 }),
            (0x1a8b310a, Csel { width: W32, rd: 10, rn: 8, rm: 11, condition: 0b0011 }),
            // Stage 2: the MMU, the kernel window and the page table builder.
            (0xb0000021, Adrp { rd: 1, offset: 0x5000 }),
            (0xd63f0120, Blr { rn: 9 }),
            (0xd508871f, TlbiVmalle1),
            (0xd5182020, Msr { register: SystemRegister::Ttbr1El1, rt: 0 }),
            (0xd5382048, Mrs { rt: 8, register: SystemRegister::TcrEl1 }),
            (0x520004a5, EorImmediate { width: W32, rd: 5, rn: 5, imm: 3 }),
            (0xf24006bf, AndsImmediate { width: W64, rd: 31, rn: 21, imm: 3 }),
            (0x9a9f0604, Csinc { width: W64, rd: 4, rn: 16, rm: 31, condition: 0 }),
            (0x7a572060, CcmpRegister { width: W32, rn: 3, rm: 23, nzcv: 0, condition: 2 }),
            (0xfa4008a4, CcmpImmediate { width: W64, rn: 5, imm: 0, nzcv: 4, condition: 0 }),
            (
                0xcb010044,
                SubShiftedRegister {
                    width: W64,
                    rd: 4,
                    rn: 2,
                    rm: 1,
                    shift: Shift::Lsl,
                    amount: 0,
                },
            ),
            (
                0x2a3703f5,
                OrnShiftedRegister {
                    width: W32,
                    rd: 21,
                    rn: 31,
                    rm: 23,
                    shift: Shift::Lsl,
                    amount: 0,
                },
            ),
            (
                0xb829690a,
                StrRegister { width: W32, rt: 10, rn: 8, rm: 9, extend: Extend::Lsl, amount: 0 },
            ),
            (0x9aca216a, ShiftVariable { width: W64, rd: 10, rn: 11, rm: 10, shift: Shift::Lsl }),
            (0x9acc252c, ShiftVariable { width: W64, rd: 12, rn: 9, rm: 12, shift: Shift::Lsr }),
            (
                0xea0b015f,
                AndsShiftedRegister {
                    width: W64,
                    rd: 31,
                    rn: 10,
                    rm: 11,
                    shift: Shift::Lsl,
                    amount: 0,
                },
            ),
            (
                0xf86a698a,
                LdrRegister { width: W64, rt: 10, rn: 12, rm: 10, extend: Extend::Lsl, amount: 0 },
            ),
        ];
        for (word, expected) in cases {
            assert_eq!(decode(word), Ok(expected), "decoding {word:#010x}");
        }
    }

    #[test]
    fn prints_instructions_in_assembler_syntax() {
        assert_eq!(decode(0xd53800a1).unwrap().to_string(), "mrs x1, MPIDR_EL1");
        assert_eq!(decode(0x92401c21).unwrap().to_string(), "and x1, x1, #0xff");
        assert_eq!(decode(0xf81f0ffe).unwrap().to_string(), "str x30, [sp, #-0x10]!");
        assert_eq!(decode(0xf800843f).unwrap().to_string(), "str xzr, [x1], #0x8");
        assert_eq!(decode(0x372fffc9).unwrap().to_string(), "tbnz w9, #5, #-0x8");
        assert_eq!(decode(0x54000062).unwrap().to_string(), "b.hs #+0xc");
        assert_eq!(decode(0xd5033f9f).unwrap().to_string(), "dsb sy");
        assert_eq!(decode(0x4a5d1821).unwrap().to_string(), "eor w1, w1, w29, lsr #6");
        assert_eq!(decode(0xd518c008).unwrap().to_string(), "msr VBAR_EL1, x8");
        assert_eq!(decode(0x1a8b310a).unwrap().to_string(), "csel w10, w8, w11, lo");
        assert_eq!(decode(0x9aca216a).unwrap().to_string(), "lslv x10, x11, x10");
        assert_eq!(decode(0xf86a698a).unwrap().to_string(), "ldr x10, [x12, x10, lsl #0]");
        assert_eq!(decode(0xd508871f).unwrap().to_string(), "tlbi vmalle1");
        assert_eq!(decode(0x7a572060).unwrap().to_string(), "ccmp w3, w23, #0x0, hs");
        assert_eq!(decode(0xfa4008a4).unwrap().to_string(), "ccmp x5, #0x0, #0x4, eq");
        assert_eq!(decode(0xb829690a).unwrap().to_string(), "str w10, [x8, x9, lsl #0]");
    }

    #[test]
    fn rejects_instructions_neos_does_not_use() {
        assert_eq!(decode(0x8a220020), Err(DecodeError::Unimplemented)); // bic x0, x1, x2
        assert_eq!(decode(0xd50b7520), Err(DecodeError::Unimplemented)); // ic ivau, x0
        assert_eq!(decode(0x9acb2949), Err(DecodeError::Unimplemented)); // asrv x9, x10, x11
        assert_eq!(decode(0x5a8b310a), Err(DecodeError::Unimplemented)); // csinv w10, w8, w11, lo
        assert_eq!(decode(0xd503207f), Err(DecodeError::Unimplemented)); // wfi
        assert_eq!(decode(0x1e202008), Err(DecodeError::Unimplemented)); // fcmp s0, s0
        assert_eq!(
            decode(0xd538d080), // mrs x0, TPIDR_EL1
            Err(DecodeError::SystemRegister { op0: 3, op1: 0, crn: 13, crm: 0, op2: 4 })
        );
    }

    #[test]
    fn rejects_unpredictable_writeback() {
        // str x1, [x1], #8
        assert!(matches!(decode(0xf8008421), Err(DecodeError::Unpredictable(_))));
    }

    #[test]
    fn expands_logical_immediates() {
        let mask = |n, imms, immr, width| decode_bit_masks(n, imms, immr, true, width).map(|m| m.0);
        assert_eq!(mask(1, 0b000111, 0, W64), Some(0xff));
        assert_eq!(mask(0, 0b000111, 0, W32), Some(0xff));
        // 0b0011 repeated, rotated right by 1: 0b1001 repeated.
        assert_eq!(mask(0, 0b111001, 1, W64), Some(0x9999_9999_9999_9999));
        // All ones is reserved.
        assert_eq!(mask(1, 0b111111, 0, W64), None);
    }
}
