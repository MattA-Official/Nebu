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

/// A system register that `MRS` can read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemRegister {
    /// The multiprocessor affinity register: which core this is.
    MpidrEl1,
    /// The current exception level.
    CurrentEl,
}

/// A decoded instruction.
///
/// Register numbers are 0 to 31. Whether 31 means the zero register or the stack
/// pointer depends on the instruction; the comments say which.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Instruction {
    /// ADR: `rd = pc + offset`. `rd` 31 is the zero register.
    Adr { rd: u8, offset: i64 },
    /// ADD (immediate): `rd = rn + imm`. 31 is SP for both registers.
    AddImmediate { width: Width, rd: u8, rn: u8, imm: u64 },
    /// SUB (immediate): `rd = rn - imm`. 31 is SP for both registers.
    SubImmediate { width: Width, rd: u8, rn: u8, imm: u64 },
    /// SUBS (immediate): `rd = rn - imm`, setting the flags. `rd` 31 is the zero
    /// register, `rn` 31 is SP.
    SubsImmediate { width: Width, rd: u8, rn: u8, imm: u64 },
    /// AND (immediate): `rd = rn & imm`. `rd` 31 is SP, `rn` 31 is the zero register.
    AndImmediate { width: Width, rd: u8, rn: u8, imm: u64 },
    /// MOVZ: `rd = imm16 << shift`. 31 is the zero register.
    Movz { width: Width, rd: u8, imm16: u16, shift: u8 },
    /// MOVK: replace bits `shift..shift + 16` of `rd` with `imm16`. 31 is the zero register.
    Movk { width: Width, rd: u8, imm16: u16, shift: u8 },
    /// BFM: copy a bitfield from `rn` into `rd`. 31 is the zero register.
    Bfm { width: Width, rd: u8, rn: u8, immr: u8, imms: u8 },
    /// ADD (shifted register): `rd = rn + (rm shifted)`. 31 is the zero register.
    AddShiftedRegister { width: Width, rd: u8, rn: u8, rm: u8, shift: Shift, amount: u8 },
    /// SUBS (shifted register): `rd = rn - (rm shifted)`, setting the flags.
    /// 31 is the zero register.
    SubsShiftedRegister { width: Width, rd: u8, rn: u8, rm: u8, shift: Shift, amount: u8 },
    /// ORR (shifted register): `rd = rn | (rm shifted)`. 31 is the zero register.
    OrrShiftedRegister { width: Width, rd: u8, rn: u8, rm: u8, shift: Shift, amount: u8 },
    /// B: branch to `pc + offset`.
    B { offset: i64 },
    /// BL: branch to `pc + offset`, putting the return address in x30.
    Bl { offset: i64 },
    /// RET: branch to the address in `rn` (normally x30).
    Ret { rn: u8 },
    /// B.cond: branch to `pc + offset` if `condition` holds.
    BCond { condition: u8, offset: i64 },
    /// CBZ: branch to `pc + offset` if `rt` is zero. 31 is the zero register.
    Cbz { width: Width, rt: u8, offset: i64 },
    /// TBZ: branch to `pc + offset` if bit `bit` of `rt` is zero.
    Tbz { rt: u8, bit: u8, offset: i64 },
    /// TBNZ: branch to `pc + offset` if bit `bit` of `rt` is one.
    Tbnz { rt: u8, bit: u8, offset: i64 },
    /// HLT: stop for an external debugger, or make a semihosting call.
    Hlt { imm16: u16 },
    /// NOP: do nothing.
    Nop,
    /// WFE: wait for an event.
    Wfe,
    /// ISB: instruction synchronisation barrier.
    Isb,
    /// MRS: read a system register into `rt`. 31 is the zero register.
    Mrs { rt: u8, register: SystemRegister },
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
    /// STRH (immediate): store the low 16 bits of `rt`.
    /// `rn` 31 is SP, `rt` 31 the zero register.
    StrhImmediate { rt: u8, rn: u8, index: Index, offset: i64 },
    /// STUR: store `rt` to `rn + offset`, with an unscaled offset.
    /// `rn` 31 is SP, `rt` 31 the zero register.
    Stur { width: Width, rt: u8, rn: u8, offset: i64 },
    /// STP: store `rt` and `rt2` to consecutive locations.
    /// `rn` 31 is SP, `rt` and `rt2` 31 the zero register.
    Stp { width: Width, rt: u8, rt2: u8, rn: u8, index: Index, offset: i64 },
}

/// Why a word could not be decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// The word is not an instruction Nebu implements.
    Unimplemented,
    /// The encoding is unallocated: real hardware raises an Undefined Instruction
    /// exception.
    Unallocated,
    /// An `MRS` of a system register Nebu does not model.
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
            if bits(word, 31, 31) == 1 {
                return Err(DecodeError::Unimplemented); // ADRP
            }
            let imm = (bits(word, 23, 5) << 2) | bits(word, 30, 29);
            Ok(Instruction::Adr { rd, offset: sign_extend(imm, 21) })
        }
        // Add/subtract (immediate).
        0b010 => {
            let imm = u64::from(bits(word, 21, 10)) << (12 * bits(word, 22, 22));
            match bits(word, 30, 29) {
                0b00 => Ok(Instruction::AddImmediate { width, rd, rn, imm }),
                0b10 => Ok(Instruction::SubImmediate { width, rd, rn, imm }),
                0b11 => Ok(Instruction::SubsImmediate { width, rd, rn, imm }),
                _ => Err(DecodeError::Unimplemented), // ADDS
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
                _ => Err(DecodeError::Unimplemented), // ORR, EOR, ANDS
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
                0b10 => Ok(Instruction::Movz { width, rd, imm16, shift }),
                0b11 => Ok(Instruction::Movk { width, rd, imm16, shift }),
                0b01 => Err(DecodeError::Unallocated),
                _ => Err(DecodeError::Unimplemented), // MOVN
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
            match bits(word, 30, 29) {
                0b01 => Ok(Instruction::Bfm { width, rd, rn, immr: immr as u8, imms: imms as u8 }),
                0b11 => Err(DecodeError::Unallocated),
                _ => Err(DecodeError::Unimplemented), // SBFM, UBFM
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
        if bits(word, 24, 24) == 1 {
            return Err(DecodeError::Unimplemented); // CBNZ
        }
        let width = Width::from_bit(bits(word, 31, 31));
        let offset = sign_extend(bits(word, 23, 5), 19) * 4;
        return Ok(Instruction::Cbz { width, rt: reg(word, 0), offset });
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
        if (opc, op2, op3, op4) == (0b0010, 0b11111, 0, 0) {
            return Ok(Instruction::Ret { rn: reg(word, 5) });
        }
        return Err(DecodeError::Unimplemented);
    }

    // Exception generation.
    if bits(word, 31, 24) == 0b1101_0100 {
        let (opc, op2, ll) = (bits(word, 23, 21), bits(word, 4, 2), bits(word, 1, 0));
        if (opc, op2, ll) == (0b010, 0b000, 0b00) {
            return Ok(Instruction::Hlt { imm16: bits(word, 20, 5) as u16 });
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
        (0, 0, 0b011, 0b0011) if rt == 31 => match op2 {
            0b110 => Ok(Instruction::Isb),
            _ => Err(DecodeError::Unimplemented),
        },
        // MRS.
        (1, 2 | 3, _, _) => {
            let register = match (op0, op1, crn, crm, op2) {
                (3, 0, 0, 0, 5) => SystemRegister::MpidrEl1,
                (3, 0, 4, 2, 2) => SystemRegister::CurrentEl,
                _ => {
                    return Err(DecodeError::SystemRegister { op0, op1, crn, crm, op2 });
                }
            };
            Ok(Instruction::Mrs { rt, register })
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

    // Load/store register pair.
    if bits(word, 29, 27) == 0b101 {
        let index = match bits(word, 24, 23) {
            0b01 => Index::Post,
            0b10 => Index::Offset,
            0b11 => Index::Pre,
            _ => return Err(DecodeError::Unimplemented), // STNP, LDNP
        };
        let width = match (bits(word, 31, 30), bits(word, 22, 22)) {
            (0b00, 0) => Width::W32,
            (0b10, 0) => Width::W64,
            _ => return Err(DecodeError::Unimplemented), // LDP, LDPSW, STGP
        };
        let rt2 = reg(word, 10);
        if index != Index::Offset && rn != 31 && (rt == rn || rt2 == rn) {
            return Err(DecodeError::Unpredictable("STP writes back to a register it stores"));
        }
        let offset = sign_extend(bits(word, 21, 15), 7) * width.bytes() as i64;
        return Ok(Instruction::Stp { width, rt, rt2, rn, index, offset });
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
                    _ => Err(DecodeError::Unimplemented),
                };
            }
            _ => return Err(DecodeError::Unimplemented),
        };

        if index != Index::Offset && rn != 31 && rt == rn {
            return Err(DecodeError::Unpredictable("writeback to the transfer register"));
        }
        return match (size, opc) {
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
        return match (bits(word, 30, 29), bits(word, 21, 21)) {
            (0b01, 0) => Ok(Instruction::OrrShiftedRegister {
                width,
                rd: reg(word, 0),
                rn: reg(word, 5),
                rm: reg(word, 16),
                shift,
                amount: amount as u8,
            }),
            _ => Err(DecodeError::Unimplemented), // AND, BIC, ORN, EOR, EON, ANDS, BICS
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
            0b11 => Ok(Instruction::SubsShiftedRegister { width, rd, rn, rm, shift, amount }),
            _ => Err(DecodeError::Unimplemented), // ADDS, SUB
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

/// The assembler names of the condition codes.
const CONDITIONS: [&str; 16] = [
    "eq", "ne", "hs", "lo", "mi", "pl", "vs", "vc", "hi", "ls", "ge", "lt", "gt", "le", "al", "nv",
];

impl fmt::Display for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use Instruction::*;
        match *self {
            Adr { rd, offset } => write!(f, "adr {}, {}", zr(rd, Width::W64), Relative(offset)),
            AddImmediate { width, rd, rn, imm } => {
                write!(f, "add {}, {}, #{imm:#x}", sp(rd, width), sp(rn, width))
            }
            SubImmediate { width, rd, rn, imm } => {
                write!(f, "sub {}, {}, #{imm:#x}", sp(rd, width), sp(rn, width))
            }
            AndImmediate { width, rd, rn, imm } => {
                write!(f, "and {}, {}, #{imm:#x}", sp(rd, width), zr(rn, width))
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
            SubsShiftedRegister { width, rd, rn, rm, shift, amount } => write!(
                f,
                "subs {}, {}, {}, {} #{amount}",
                zr(rd, width),
                zr(rn, width),
                zr(rm, width),
                shift_name(shift)
            ),
            OrrShiftedRegister { width, rd, rn, rm, shift, amount } => write!(
                f,
                "orr {}, {}, {}, {} #{amount}",
                zr(rd, width),
                zr(rn, width),
                zr(rm, width),
                shift_name(shift)
            ),
            B { offset } => write!(f, "b {}", Relative(offset)),
            Bl { offset } => write!(f, "bl {}", Relative(offset)),
            Ret { rn } => write!(f, "ret {}", zr(rn, Width::W64)),
            BCond { condition, offset } => {
                write!(f, "b.{} {}", CONDITIONS[usize::from(condition)], Relative(offset))
            }
            Cbz { width, rt, offset } => write!(f, "cbz {}, {}", zr(rt, width), Relative(offset)),
            Tbz { rt, bit, offset } => {
                let width = if bit >= 32 { Width::W64 } else { Width::W32 };
                write!(f, "tbz {}, #{bit}, {}", zr(rt, width), Relative(offset))
            }
            Tbnz { rt, bit, offset } => {
                let width = if bit >= 32 { Width::W64 } else { Width::W32 };
                write!(f, "tbnz {}, #{bit}, {}", zr(rt, width), Relative(offset))
            }
            Hlt { imm16 } => write!(f, "hlt #{imm16:#x}"),
            Nop => write!(f, "nop"),
            Wfe => write!(f, "wfe"),
            Isb => write!(f, "isb"),
            Mrs { rt, register } => {
                let name = match register {
                    SystemRegister::MpidrEl1 => "MPIDR_EL1",
                    SystemRegister::CurrentEl => "CurrentEL",
                };
                write!(f, "mrs {}, {name}", zr(rt, Width::W64))
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
    }

    #[test]
    fn rejects_instructions_neos_does_not_use() {
        assert_eq!(decode(0xd63f0100), Err(DecodeError::Unimplemented)); // blr x8
        assert_eq!(decode(0xb5000061), Err(DecodeError::Unimplemented)); // cbnz x1
        assert_eq!(decode(0xd503207f), Err(DecodeError::Unimplemented)); // wfi
        assert_eq!(decode(0x1e202008), Err(DecodeError::Unimplemented)); // fcmp s0, s0
        assert_eq!(
            decode(0xd5381000), // mrs x0, SCTLR_EL1
            Err(DecodeError::SystemRegister { op0: 3, op1: 0, crn: 1, crm: 0, op2: 0 })
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
