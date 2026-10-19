//! The CPU: one Cortex-A53 core running A64 code (ADR 0004).
//!
//! The pseudocode for each instruction is in the Arm Architecture Reference Manual
//! (Arm DDI 0487), on the instruction's own page.

use std::fmt;

use crate::bus::{Bus, BusError};
use crate::decode::decode_bit_masks;
use crate::decode::{
    DecodeError, Extend, Index, Instruction, Shift, SystemRegister, Width, decode,
};

/// MPIDR_EL1 for core 0, as QEMU reports it: bit 31 is RES1 and the core number is
/// in the low byte.
const MPIDR_CORE_0: u64 = 0x8000_0000;

/// The semihosting trap: `hlt #0xf000` (ADR 0006).
const SEMIHOSTING_HLT: u16 = 0xF000;
/// Semihosting operation: end the run.
const SYS_EXIT: u64 = 0x18;
/// SYS_EXIT reason for a normal exit with an exit code.
const ADP_STOPPED_APPLICATION_EXIT: u64 = 0x2_0026;

/// The condition flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    /// Negative.
    pub n: bool,
    /// Zero.
    pub z: bool,
    /// Carry.
    pub c: bool,
    /// Overflow.
    pub v: bool,
}

/// What happened when the CPU executed an instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Carry on with the next instruction.
    Continue,
    /// The core is waiting for an event (`wfe`).
    Asleep,
    /// The software asked to end the run with this exit code (semihosting).
    Exit(i32),
}

/// Something Nebu cannot emulate, which stops the run (ADR 0001).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// An instruction could not be decoded.
    Decode { word: u32, error: DecodeError },
    /// A memory access failed.
    Memory { address: u64, write: bool, error: BusError },
    /// A load or store used SP as its base while SP was not 16-byte aligned. Whether
    /// hardware faults depends on SCTLR_ELx.SA, which Nebu does not model yet.
    StackAlignment { sp: u64 },
    /// A system register access that hardware would trap or treat as undefined at
    /// this exception level. Nebu does not model the resulting exception.
    SystemRegisterAccess { register: SystemRegister, el: u8, write: bool },
    /// The software asked for something the architecture allows but Nebu does not
    /// model, such as turning on the MMU.
    Unsupported(&'static str),
    /// `hlt` with semihosting off: the core stops for an external debugger.
    Halt { imm16: u16 },
    /// A semihosting operation Nebu does not implement.
    Semihosting { operation: u64 },
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fault::Decode { word, error } => {
                write!(f, "cannot run instruction {word:#010x}: {error}")
            }
            Fault::Memory { address, write, error } => {
                let kind = if *write { "write to" } else { "read from" };
                write!(f, "{kind} {address:#x} failed: {error}")
            }
            Fault::StackAlignment { sp } => {
                write!(f, "SP-based access with SP = {sp:#x}, which is not 16-byte aligned")
            }
            Fault::SystemRegisterAccess { register, el, write } => {
                let kind = if *write { "writing" } else { "reading" };
                write!(
                    f,
                    "{kind} {} at EL{el} is not allowed (hardware would trap)",
                    register.name()
                )
            }
            Fault::Unsupported(what) => write!(f, "not implemented: {what}"),
            Fault::Halt { imm16 } => {
                write!(f, "hlt #{imm16:#x} with semihosting off (the core halts for a debugger)")
            }
            Fault::Semihosting { operation } => {
                write!(f, "semihosting operation {operation:#x} not implemented")
            }
        }
    }
}

/// The system registers Nebu models, apart from the read-only identification ones.
#[derive(Clone, Copy, Debug, Default)]
struct SystemRegisters {
    sctlr_el1: u64,
    spsr_el1: u64,
    elr_el1: u64,
    esr_el1: u64,
    far_el1: u64,
    vbar_el1: u64,
    hcr_el2: u64,
    cnthctl_el2: u64,
    cntvoff_el2: u64,
    spsr_el2: u64,
    elr_el2: u64,
}

/// HCR_EL2.RW: EL1 runs AArch64.
const HCR_EL2_RW: u64 = 1 << 31;
/// SCTLR_ELx.M: the MMU is on.
const SCTLR_M: u64 = 1;

/// Exception classes, the top 6 bits of ESR_ELx.
const EC_UNKNOWN: u64 = 0x00;
const EC_BRK: u64 = 0x3C;
/// ESR_ELx.IL: the instruction that caused the exception was 32 bits long.
const ESR_IL: u64 = 1 << 25;

/// The state of one core.
pub struct Cpu {
    /// General-purpose registers x0 to x30.
    pub x: [u64; 31],
    /// The stack pointers SP_EL0, SP_EL1 and SP_EL2.
    sp_el: [u64; 3],
    /// PSTATE.SP: whether the current EL uses its own stack pointer (SP_ELx, "h")
    /// rather than SP_EL0 ("t").
    pub spsel: bool,
    /// The program counter.
    pub pc: u64,
    /// The condition flags.
    pub flags: Flags,
    /// The current exception level.
    pub el: u8,
    /// PSTATE.DAIF: which exceptions are masked. Nothing raises interrupts yet, so
    /// this is only stored and restored.
    daif: u8,
    sys: SystemRegisters,
    /// The event register, which `wfe` waits on.
    event: bool,
    /// Whether `hlt #0xf000` makes semihosting calls.
    semihosting: bool,
}

impl Cpu {
    /// Creates core 0 in the state QEMU's `raspi3b` leaves it in when it jumps to the
    /// kernel at `entry` (ADR 0006).
    ///
    /// QEMU first runs a small boot stub at address 0, which leaves x0 holding
    /// 0x100 (where a device tree or ATAGS would be), x4 holding the entry point and
    /// the Z flag set. Everything else is zero. This was read from QEMU 8.2's
    /// `-d cpu` log at the first instruction of the kernel.
    pub fn new(entry: u64, semihosting: bool) -> Self {
        let mut x = [0; 31];
        x[0] = 0x100;
        x[4] = entry;
        Cpu {
            x,
            sp_el: [0; 3],
            spsel: true,
            pc: entry,
            flags: Flags { z: true, ..Flags::default() },
            el: 2,
            daif: 0b1111,
            sys: SystemRegisters::default(),
            event: false,
            semihosting,
        }
    }

    /// The stack pointer in use: SP_ELx for the current EL, or SP_EL0.
    pub fn sp(&self) -> u64 {
        self.sp_el[self.sp_index()]
    }

    fn sp_index(&self) -> usize {
        if self.spsel { usize::from(self.el) } else { 0 }
    }

    /// PSTATE as SPSR_ELx stores it: the flags, DAIF, and the mode in M[3:0].
    fn pstate(&self) -> u64 {
        let Flags { n, z, c, v } = self.flags;
        u64::from(n) << 31
            | u64::from(z) << 30
            | u64::from(c) << 29
            | u64::from(v) << 28
            | u64::from(self.daif) << 6
            | u64::from(self.el) << 2
            | u64::from(self.spsel)
    }

    /// Takes a synchronous exception with class `ec` and syndrome `iss`, returning to
    /// `preferred_return` afterwards. Returns the address of the vector to run.
    fn take_exception(&mut self, ec: u64, iss: u64, preferred_return: u64) -> Result<u64, Fault> {
        // With HCR_EL2.TGE clear, exceptions from EL0 and EL1 go to EL1. Exceptions at
        // EL2 would go to EL2's own vectors, which nothing uses.
        let offset = match (self.el, self.spsel) {
            (0, _) => 0x400,
            (1, false) => 0x000,
            (1, true) => 0x200,
            _ => return Err(Fault::Unsupported("taking an exception at EL2")),
        };
        self.sys.spsr_el1 = self.pstate();
        self.sys.elr_el1 = preferred_return;
        self.sys.esr_el1 = ec << 26 | ESR_IL | iss;
        self.el = 1;
        self.spsel = true;
        self.daif = 0b1111;
        Ok(self.sys.vbar_el1.wrapping_add(offset))
    }

    /// ERET: restores PSTATE from the current EL's SPSR and returns the address in its
    /// ELR.
    fn exception_return(&mut self) -> Result<u64, Fault> {
        let (spsr, elr) = match self.el {
            1 => (self.sys.spsr_el1, self.sys.elr_el1),
            2 => (self.sys.spsr_el2, self.sys.elr_el2),
            _ => return Err(Fault::Unsupported("eret at EL0 or EL3")),
        };
        let mode = spsr & 0b1_1111;
        let target = (mode >> 2 & 0b11) as u8;
        let spsel = mode & 1 == 1;
        // The architecture calls these illegal exception returns, which set PSTATE.IL
        // instead of changing level. Nebu does not model that.
        if mode & 0b1_0000 != 0 {
            return Err(Fault::Unsupported("returning to AArch32"));
        }
        if target > self.el || mode & 0b10 != 0 || (target == 0 && spsel) {
            return Err(Fault::Unsupported("an illegal exception return"));
        }
        if self.el == 2 && target == 1 && self.sys.hcr_el2 & HCR_EL2_RW == 0 {
            return Err(Fault::Unsupported("running EL1 in AArch32 (HCR_EL2.RW is 0)"));
        }
        self.flags = Flags {
            n: spsr >> 31 & 1 == 1,
            z: spsr >> 30 & 1 == 1,
            c: spsr >> 29 & 1 == 1,
            v: spsr >> 28 & 1 == 1,
        };
        self.daif = (spsr >> 6 & 0b1111) as u8;
        self.el = target;
        self.spsel = spsel;
        Ok(elr)
    }

    /// The lowest exception level that may access `register`.
    fn minimum_el(register: SystemRegister) -> u8 {
        use SystemRegister::*;
        match register {
            MpidrEl1 | CurrentEl | SctlrEl1 | SpsrEl1 | ElrEl1 | SpEl0 | EsrEl1 | FarEl1
            | VbarEl1 => 1,
            HcrEl2 | CnthctlEl2 | CntvoffEl2 | SpsrEl2 | ElrEl2 | SpEl1 => 2,
        }
    }

    /// MRS: reads a system register.
    fn read_system_register(&self, register: SystemRegister) -> Result<u64, Fault> {
        if self.el < Self::minimum_el(register) {
            return Err(Fault::SystemRegisterAccess { register, el: self.el, write: false });
        }
        use SystemRegister::*;
        Ok(match register {
            MpidrEl1 => MPIDR_CORE_0,
            CurrentEl => u64::from(self.el) << 2,
            SctlrEl1 => self.sys.sctlr_el1,
            SpsrEl1 => self.sys.spsr_el1,
            ElrEl1 => self.sys.elr_el1,
            SpEl0 => self.sp_el[0],
            EsrEl1 => self.sys.esr_el1,
            FarEl1 => self.sys.far_el1,
            VbarEl1 => self.sys.vbar_el1,
            HcrEl2 => self.sys.hcr_el2,
            CnthctlEl2 => self.sys.cnthctl_el2,
            CntvoffEl2 => self.sys.cntvoff_el2,
            SpsrEl2 => self.sys.spsr_el2,
            ElrEl2 => self.sys.elr_el2,
            SpEl1 => self.sp_el[1],
        })
    }

    /// MSR (register): writes a system register.
    fn write_system_register(&mut self, register: SystemRegister, value: u64) -> Result<(), Fault> {
        if self.el < Self::minimum_el(register) {
            return Err(Fault::SystemRegisterAccess { register, el: self.el, write: true });
        }
        use SystemRegister::*;
        match register {
            MpidrEl1 | CurrentEl | EsrEl1 | FarEl1 => {
                // Writable on hardware in the case of ESR and FAR, but nothing should.
                return Err(Fault::SystemRegisterAccess { register, el: self.el, write: true });
            }
            SctlrEl1 => {
                if value & SCTLR_M != 0 {
                    return Err(Fault::Unsupported("turning on the MMU"));
                }
                self.sys.sctlr_el1 = value;
            }
            SpsrEl1 => self.sys.spsr_el1 = value,
            ElrEl1 => self.sys.elr_el1 = value,
            SpEl0 => self.sp_el[0] = value,
            // The low 11 bits are RES0: the table is 2 KiB-aligned.
            VbarEl1 => self.sys.vbar_el1 = value & !0x7FF,
            HcrEl2 => {
                if value & !HCR_EL2_RW != 0 {
                    return Err(Fault::Unsupported("HCR_EL2 bits other than RW"));
                }
                self.sys.hcr_el2 = value;
            }
            // Nebu has no timers yet, so these are only stored.
            CnthctlEl2 => self.sys.cnthctl_el2 = value,
            CntvoffEl2 => self.sys.cntvoff_el2 = value,
            SpsrEl2 => self.sys.spsr_el2 = value,
            ElrEl2 => self.sys.elr_el2 = value,
            SpEl1 => self.sp_el[1] = value,
        }
        Ok(())
    }

    /// Reads register `n` (31 is the zero register) at `width`.
    fn read(&self, n: u8, width: Width) -> u64 {
        if n == 31 { 0 } else { self.x[usize::from(n)] & width.mask() }
    }

    /// Writes register `n` (31 is the zero register). 32-bit writes clear the top half.
    fn write(&mut self, n: u8, width: Width, value: u64) {
        if n != 31 {
            self.x[usize::from(n)] = value & width.mask();
        }
    }

    /// Reads register `n`, where 31 is the stack pointer.
    fn read_sp(&self, n: u8, width: Width) -> u64 {
        if n == 31 { self.sp() & width.mask() } else { self.read(n, width) }
    }

    /// Writes register `n`, where 31 is the stack pointer.
    fn write_sp(&mut self, n: u8, width: Width, value: u64) {
        if n == 31 {
            let index = self.sp_index();
            self.sp_el[index] = value & width.mask();
        } else {
            self.write(n, width, value)
        }
    }

    /// The architecture's `ConditionHolds`.
    fn condition_holds(&self, condition: u8) -> bool {
        let Flags { n, z, c, v } = self.flags;
        let result = match condition >> 1 {
            0b000 => z,
            0b001 => c,
            0b010 => n,
            0b011 => v,
            0b100 => c && !z,
            0b101 => n == v,
            0b110 => n == v && !z,
            _ => true,
        };
        if condition & 1 == 1 && condition != 0b1111 { !result } else { result }
    }

    /// Computes the base address of a load or store whose base register is `rn`.
    fn base(&self, rn: u8) -> Result<u64, Fault> {
        if rn == 31 && !self.sp().is_multiple_of(16) {
            return Err(Fault::StackAlignment { sp: self.sp() });
        }
        Ok(self.read_sp(rn, Width::W64))
    }

    /// Fetches, decodes and executes one instruction. With `trace`, prints it first.
    pub fn step(&mut self, bus: &mut Bus, trace: bool) -> Result<Step, Fault> {
        let pc = self.pc;
        let word =
            bus.fetch(pc).map_err(|error| Fault::Memory { address: pc, write: false, error })?;
        let instruction = decode(word).map_err(|error| Fault::Decode { word, error })?;
        if trace {
            eprintln!("{pc:#010x}: {word:08x}  {instruction}");
        }

        let mut next = pc.wrapping_add(4);
        let mut result = Step::Continue;
        let branch = |offset: i64| pc.wrapping_add_signed(offset);

        use Instruction::*;
        match instruction {
            Adr { rd, offset } => self.write(rd, Width::W64, branch(offset)),
            AddImmediate { width, rd, rn, imm } => {
                let value = self.read_sp(rn, width).wrapping_add(imm);
                self.write_sp(rd, width, value);
            }
            SubImmediate { width, rd, rn, imm } => {
                let value = self.read_sp(rn, width).wrapping_sub(imm);
                self.write_sp(rd, width, value);
            }
            AndImmediate { width, rd, rn, imm } => {
                let value = self.read(rn, width) & imm;
                self.write_sp(rd, width, value);
            }
            OrrImmediate { width, rd, rn, imm } => {
                let value = self.read(rn, width) | imm;
                self.write_sp(rd, width, value);
            }
            Movn { width, rd, imm16, shift } => {
                self.write(rd, width, !(u64::from(imm16) << shift));
            }
            Movz { width, rd, imm16, shift } => self.write(rd, width, u64::from(imm16) << shift),
            Movk { width, rd, imm16, shift } => {
                let kept = self.read(rd, width) & !(0xFFFF << shift);
                self.write(rd, width, kept | u64::from(imm16) << shift);
            }
            Bfm { width, rd, rn, immr, imms } => {
                let (wmask, tmask) = bitfield_masks(immr, imms, width);
                let dst = self.read(rd, width);
                let src = self.read(rn, width);
                let rotated = rotate_right(src, u32::from(immr), width);
                let bottom = (dst & !wmask) | (rotated & wmask);
                self.write(rd, width, (dst & !tmask) | (bottom & tmask));
            }
            Sbfm { width, rd, rn, immr, imms } => {
                let (wmask, tmask) = bitfield_masks(immr, imms, width);
                let src = self.read(rn, width);
                let bottom = rotate_right(src, u32::from(immr), width) & wmask;
                // Every bit above the field is a copy of the field's top bit, src<imms>.
                let top = if src >> imms & 1 == 1 { width.mask() } else { 0 };
                self.write(rd, width, (top & !tmask) | (bottom & tmask));
            }
            Ubfm { width, rd, rn, immr, imms } => {
                let (wmask, tmask) = bitfield_masks(immr, imms, width);
                let src = self.read(rn, width);
                let bottom = rotate_right(src, u32::from(immr), width) & wmask;
                self.write(rd, width, bottom & tmask);
            }
            AddsImmediate { width, rd, rn, imm } => {
                let operand1 = self.read_sp(rn, width);
                let (value, flags) = add_with_carry(operand1, imm, false, width);
                self.write(rd, width, value);
                self.flags = flags;
            }
            SubsImmediate { width, rd, rn, imm } => {
                let operand1 = self.read_sp(rn, width);
                let (value, flags) = add_with_carry(operand1, !imm & width.mask(), true, width);
                self.write(rd, width, value);
                self.flags = flags;
            }
            AndShiftedRegister { width, rd, rn, rm, shift, amount } => {
                let operand2 = shift_register(self.read(rm, width), shift, amount, width);
                self.write(rd, width, self.read(rn, width) & operand2);
            }
            OrrShiftedRegister { width, rd, rn, rm, shift, amount } => {
                let operand2 = shift_register(self.read(rm, width), shift, amount, width);
                self.write(rd, width, self.read(rn, width) | operand2);
            }
            EorShiftedRegister { width, rd, rn, rm, shift, amount } => {
                let operand2 = shift_register(self.read(rm, width), shift, amount, width);
                self.write(rd, width, self.read(rn, width) ^ operand2);
            }
            AndsShiftedRegister { width, rd, rn, rm, shift, amount } => {
                let operand2 = shift_register(self.read(rm, width), shift, amount, width);
                let value = self.read(rn, width) & operand2;
                self.write(rd, width, value);
                let top = width.bits() - 1;
                self.flags = Flags { n: value >> top & 1 == 1, z: value == 0, c: false, v: false };
            }
            ShiftVariable { width, rd, rn, rm, shift } => {
                // The shift amount is taken modulo the register width.
                let amount = (self.read(rm, width) % u64::from(width.bits())) as u8;
                self.write(rd, width, shift_register(self.read(rn, width), shift, amount, width));
            }
            Csel { width, rd, rn, rm, condition } => {
                let source = if self.condition_holds(condition) { rn } else { rm };
                self.write(rd, width, self.read(source, width));
            }
            AddShiftedRegister { width, rd, rn, rm, shift, amount } => {
                let operand2 = shift_register(self.read(rm, width), shift, amount, width);
                self.write(rd, width, self.read(rn, width).wrapping_add(operand2));
            }
            SubsShiftedRegister { width, rd, rn, rm, shift, amount } => {
                let operand1 = self.read(rn, width);
                let operand2 = shift_register(self.read(rm, width), shift, amount, width);
                let (value, flags) =
                    add_with_carry(operand1, !operand2 & width.mask(), true, width);
                self.write(rd, width, value);
                self.flags = flags;
            }
            B { offset } => next = branch(offset),
            Bl { offset } => {
                self.write(30, Width::W64, pc.wrapping_add(4));
                next = branch(offset);
            }
            Br { rn } | Ret { rn } => next = self.read(rn, Width::W64),
            BCond { condition, offset } => {
                if self.condition_holds(condition) {
                    next = branch(offset);
                }
            }
            Cbnz { width, rt, offset } => {
                if self.read(rt, width) != 0 {
                    next = branch(offset);
                }
            }
            Cbz { width, rt, offset } => {
                if self.read(rt, width) == 0 {
                    next = branch(offset);
                }
            }
            Tbz { rt, bit, offset } => {
                if self.read(rt, Width::W64) & (1 << bit) == 0 {
                    next = branch(offset);
                }
            }
            Tbnz { rt, bit, offset } => {
                if self.read(rt, Width::W64) & (1 << bit) != 0 {
                    next = branch(offset);
                }
            }
            Hlt { imm16 } => {
                if !(self.semihosting && imm16 == SEMIHOSTING_HLT) {
                    return Err(Fault::Halt { imm16 });
                }
                result = self.semihosting_call(bus)?;
            }
            Brk { imm16 } => next = self.take_exception(EC_BRK, u64::from(imm16), pc)?,
            Udf { .. } => next = self.take_exception(EC_UNKNOWN, 0, pc)?,
            Eret => next = self.exception_return()?,
            // No pipeline or caches are modelled, so a barrier has nothing to wait for
            // and there is no instruction cache to invalidate.
            Nop | Isb | Dsb { .. } | IcIallu => {}
            Wfe => {
                if self.event {
                    self.event = false;
                } else {
                    result = Step::Asleep;
                }
            }
            Mrs { rt, register } => {
                let value = self.read_system_register(register)?;
                self.write(rt, Width::W64, value);
            }
            Msr { register, rt } => {
                self.write_system_register(register, self.read(rt, Width::W64))?;
            }
            LdrLiteral { width, rt, offset } => {
                let value = load(bus, branch(offset), width.bytes())?;
                self.write(rt, width, value);
            }
            LdrImmediate { width, rt, rn, index, offset } => {
                let (address, writeback) = self.address(rn, index, offset)?;
                let value = load(bus, address, width.bytes())?;
                if let Some(base) = writeback {
                    self.write_sp(rn, Width::W64, base);
                }
                self.write(rt, width, value);
            }
            StrImmediate { width, rt, rn, index, offset } => {
                let (address, writeback) = self.address(rn, index, offset)?;
                store(bus, address, width.bytes(), self.read(rt, width))?;
                if let Some(base) = writeback {
                    self.write_sp(rn, Width::W64, base);
                }
            }
            LdrbImmediate { rt, rn, index, offset } => {
                let (address, writeback) = self.address(rn, index, offset)?;
                let value = load(bus, address, 1)?;
                if let Some(base) = writeback {
                    self.write_sp(rn, Width::W64, base);
                }
                self.write(rt, Width::W32, value);
            }
            LdrbRegister { rt, rn, rm, extend } => {
                let address = self.base(rn)?.wrapping_add(self.extend_register(rm, extend));
                let value = load(bus, address, 1)?;
                self.write(rt, Width::W32, value);
            }
            LdrRegister { width, rt, rn, rm, extend, amount } => {
                let offset = self.extend_register(rm, extend) << amount;
                let address = self.base(rn)?.wrapping_add(offset);
                let value = load(bus, address, width.bytes())?;
                self.write(rt, width, value);
            }
            StrbImmediate { rt, rn, index, offset } => {
                let (address, writeback) = self.address(rn, index, offset)?;
                store(bus, address, 1, self.read(rt, Width::W32))?;
                if let Some(base) = writeback {
                    self.write_sp(rn, Width::W64, base);
                }
            }
            StrhImmediate { rt, rn, index, offset } => {
                let (address, writeback) = self.address(rn, index, offset)?;
                store(bus, address, 2, self.read(rt, Width::W32))?;
                if let Some(base) = writeback {
                    self.write_sp(rn, Width::W64, base);
                }
            }
            Stur { width, rt, rn, offset } => {
                let (address, _) = self.address(rn, Index::Offset, offset)?;
                store(bus, address, width.bytes(), self.read(rt, width))?;
            }
            Stp { width, rt, rt2, rn, index, offset } => {
                let (address, writeback) = self.address(rn, index, offset)?;
                store(bus, address, width.bytes(), self.read(rt, width))?;
                store(
                    bus,
                    address.wrapping_add(width.bytes()),
                    width.bytes(),
                    self.read(rt2, width),
                )?;
                if let Some(base) = writeback {
                    self.write_sp(rn, Width::W64, base);
                }
            }
            Ldp { width, rt, rt2, rn, index, offset } => {
                let (address, writeback) = self.address(rn, index, offset)?;
                let first = load(bus, address, width.bytes())?;
                let second = load(bus, address.wrapping_add(width.bytes()), width.bytes())?;
                if let Some(base) = writeback {
                    self.write_sp(rn, Width::W64, base);
                }
                self.write(rt, width, first);
                self.write(rt2, width, second);
            }
        }

        self.pc = next;
        Ok(result)
    }

    /// The architecture's `ExtendReg` for a register offset, before any shift.
    fn extend_register(&self, rm: u8, extend: Extend) -> u64 {
        match extend {
            Extend::Uxtw => self.read(rm, Width::W32),
            Extend::Sxtw => self.read(rm, Width::W32) as u32 as i32 as i64 as u64,
            Extend::Lsl | Extend::Sxtx => self.read(rm, Width::W64),
        }
    }

    /// Works out the address a load or store accesses, and the new base register
    /// value if it writes back.
    fn address(&self, rn: u8, index: Index, offset: i64) -> Result<(u64, Option<u64>), Fault> {
        let base = self.base(rn)?;
        let offset_base = base.wrapping_add_signed(offset);
        Ok(match index {
            Index::Offset => (offset_base, None),
            Index::Pre => (offset_base, Some(offset_base)),
            Index::Post => (base, Some(offset_base)),
        })
    }

    /// Performs the semihosting call described by x0 and x1.
    fn semihosting_call(&mut self, bus: &mut Bus) -> Result<Step, Fault> {
        // The operation number is in w0.
        let operation = self.read(0, Width::W32);
        if operation != SYS_EXIT {
            return Err(Fault::Semihosting { operation });
        }
        // On AArch64, x1 points to a block holding the reason and the exit code.
        let block = self.read(1, Width::W64);
        let reason = load(bus, block, 8)?;
        let code = load(bus, block.wrapping_add(8), 8)?;
        // Like QEMU, any reason other than a normal exit counts as failure.
        let code = if reason == ADP_STOPPED_APPLICATION_EXIT { code as i32 } else { 1 };
        Ok(Step::Exit(code))
    }
}

fn load(bus: &mut Bus, address: u64, size: u64) -> Result<u64, Fault> {
    bus.read(address, size).map_err(|error| Fault::Memory { address, write: false, error })
}

fn store(bus: &mut Bus, address: u64, size: u64, value: u64) -> Result<(), Fault> {
    bus.write(address, size, value).map_err(|error| Fault::Memory { address, write: true, error })
}

/// The `(wmask, tmask)` pair for a bitfield instruction's `immr` and `imms`.
fn bitfield_masks(immr: u8, imms: u8, width: Width) -> (u64, u64) {
    let n = u32::from(width == Width::W64);
    decode_bit_masks(n, u32::from(imms), u32::from(immr), false, width)
        .expect("the decoder only accepts valid bitfield encodings")
}

/// Rotates the low `width` bits of `value` right by `amount`.
fn rotate_right(value: u64, amount: u32, width: Width) -> u64 {
    match width {
        Width::W32 => u64::from((value as u32).rotate_right(amount)),
        Width::W64 => value.rotate_right(amount),
    }
}

/// The architecture's `ShiftReg`.
fn shift_register(value: u64, shift: Shift, amount: u8, width: Width) -> u64 {
    let amount = u32::from(amount);
    let result = match (shift, width) {
        (Shift::Lsl, _) => value << amount,
        (Shift::Lsr, _) => value >> amount,
        (Shift::Asr, Width::W32) => ((value as u32 as i32) >> amount) as u32 as u64,
        (Shift::Asr, Width::W64) => ((value as i64) >> amount) as u64,
        (Shift::Ror, _) => rotate_right(value, amount, width),
    };
    result & width.mask()
}

/// The architecture's `AddWithCarry`: returns `x + y + carry` and the resulting flags.
fn add_with_carry(x: u64, y: u64, carry: bool, width: Width) -> (u64, Flags) {
    let bits = width.bits();
    let signed = |value: u64| -> i128 {
        let shift = 128 - bits;
        (i128::from(value) << shift) >> shift
    };
    let unsigned_sum = u128::from(x) + u128::from(y) + u128::from(carry);
    let signed_sum = signed(x) + signed(y) + i128::from(carry);
    let result = (unsigned_sum as u64) & width.mask();
    let flags = Flags {
        n: result >> (bits - 1) & 1 == 1,
        z: result == 0,
        c: u128::from(result) != unsigned_sum,
        v: signed(result) != signed_sum,
    };
    (result, flags)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pl011::Pl011;

    /// A UART that discards its output and never receives anything.
    fn uart() -> Pl011 {
        Pl011::new(Box::new(std::io::sink()), std::sync::mpsc::channel().1)
    }

    #[test]
    fn add_with_carry_sets_the_flags() {
        let flags = |n, z, c, v| Flags { n, z, c, v };
        // cmp 5, 5: 5 + !5 + 1 = 0, with carry out.
        assert_eq!(add_with_carry(5, !5, true, Width::W64), (0, flags(false, true, true, false)));
        // cmp 3, 5: negative, no carry (a borrow happened).
        assert_eq!(
            add_with_carry(3, !5, true, Width::W64),
            ((-2i64) as u64, flags(true, false, false, false))
        );
        // 0x7fffffff + 1 overflows in 32 bits.
        assert_eq!(
            add_with_carry(0x7FFF_FFFF, 1, false, Width::W32),
            (0x8000_0000, flags(true, false, false, true))
        );
    }

    #[test]
    fn conditions_follow_the_flags() {
        let mut cpu = Cpu::new(0, false);
        cpu.flags = Flags { n: false, z: false, c: true, v: false };
        assert!(cpu.condition_holds(0b0010)); // hs
        assert!(!cpu.condition_holds(0b0011)); // lo
        assert!(cpu.condition_holds(0b1000)); // hi
        assert!(cpu.condition_holds(0b1110)); // al
        assert!(cpu.condition_holds(0b1111)); // nv also always holds
    }

    /// Runs `program` from address 0x80000 until it stops.
    fn run(program: &[u32], steps: usize) -> (Cpu, Bus) {
        let mut bus = Bus::new(uart());
        let bytes: Vec<u8> = program.iter().flat_map(|word| word.to_le_bytes()).collect();
        bus.load(0x80000, &bytes).unwrap();
        let mut cpu = Cpu::new(0x80000, true);
        for _ in 0..steps {
            cpu.step(&mut bus, false).unwrap();
        }
        (cpu, bus)
    }

    #[test]
    fn bfxil_extracts_the_exception_level() {
        // mrs x10, CurrentEL; mov w9, #0x30; bfxil w9, w10, #2, #2
        let (cpu, _) = run(&[0xd538424a, 0x52800609, 0x33020d49], 3);
        assert_eq!(cpu.x[9], 0x32);
    }

    #[test]
    fn stores_with_writeback_update_the_base() {
        // movz x1, #0x1000; str xzr, [x1], #8; movz x2, #0x2000; str x1, [x2, #16]!
        let (cpu, mut bus) = run(&[0xd2820001, 0xf800843f, 0xd2840002, 0xf8010c41], 4);
        assert_eq!(cpu.x[1], 0x1008);
        assert_eq!(cpu.x[2], 0x2010);
        assert_eq!(bus.read(0x2010, 8), Ok(0x1008));
    }

    #[test]
    fn bitfield_moves_sign_and_zero_extend() {
        // movn w27, #0; movz w1, #0x81; sxtb w2, w1; lsl w3, w1, #4
        let (cpu, _) = run(&[0x1280001b, 0x52801021, 0x13001c22, 0x531c6c23], 4);
        assert_eq!(cpu.x[27], 0xFFFF_FFFF);
        assert_eq!(cpu.x[2], 0xFFFF_FF81);
        assert_eq!(cpu.x[3], 0x810);
    }

    #[test]
    fn literal_loads_and_register_branches() {
        // ldr x1, #8; br x1; then the literal 0x80010
        let (cpu, _) = run(&[0x58000041, 0xd61f0020, 0x0008_0010, 0], 2);
        assert_eq!(cpu.x[1], 0x80010);
        assert_eq!(cpu.pc, 0x80010);
    }

    #[test]
    fn byte_stores_and_pair_loads() {
        // movz x1, #0x1000; movz w2, #0x1ab; strb w2, [x1]; ldp x3, x4, [x1]
        let (cpu, _) = run(&[0xd2820001, 0x52803562, 0x39000022, 0xa9401023], 4);
        assert_eq!(cpu.x[3], 0xab);
        assert_eq!(cpu.x[4], 0);
    }

    #[test]
    fn cmn_sets_the_flags_of_an_addition() {
        // cmn w5, #1 with w5 = -1: the sum wraps to zero with a carry out.
        let mut bus = Bus::new(uart());
        bus.load(0x80000, &0x310004bfu32.to_le_bytes()).unwrap();
        let mut cpu = Cpu::new(0x80000, false);
        cpu.x[5] = 0xFFFF_FFFF;
        cpu.step(&mut bus, false).unwrap();
        assert_eq!(cpu.flags, Flags { n: false, z: true, c: true, v: false });
    }

    #[test]
    fn dropping_to_el1_and_taking_exceptions() {
        // At EL2: mov x1, #(1 << 31); msr hcr_el2, x1; mov x1, #0x3c5; msr spsr_el2, x1;
        // adr x1, +12; msr elr_el2, x1; eret. Then at EL1, at 0x8001c:
        // movz x8, #0x800 (the vectors); msr vbar_el1, x8; brk #0x7.
        let program = [
            0xd2b00001, 0xd51c1101, 0xd28078a1, 0xd51c4001, 0x10000061, 0xd51c4021, 0xd69f03e0,
            0xd2810008, 0xd518c008, 0xd42000e0,
        ];
        let (cpu, _) = run(&program, 10);
        assert_eq!((cpu.el, cpu.spsel), (1, true));
        // The breakpoint at 0x80024 went to the "current EL with SP_ELx" vector.
        assert_eq!(cpu.pc, 0x800 + 0x200);
        assert_eq!(cpu.sys.elr_el1, 0x80024);
        assert_eq!(cpu.sys.esr_el1, 0xf200_0007);
        assert_eq!(cpu.sys.spsr_el1, 0x3c5);
    }

    #[test]
    fn eret_restores_pstate() {
        let mut bus = Bus::new(uart());
        bus.load(0x80000, &0xd69f03e0u32.to_le_bytes()).unwrap(); // eret
        let mut cpu = Cpu::new(0x80000, false);
        cpu.el = 1;
        cpu.sys.elr_el1 = 0x1234;
        cpu.sys.spsr_el1 = 0x6000_0000; // Z and C set, EL0t, nothing masked
        cpu.step(&mut bus, false).unwrap();
        assert_eq!((cpu.el, cpu.spsel, cpu.pc, cpu.daif), (0, false, 0x1234, 0));
        assert_eq!(cpu.flags, Flags { n: false, z: true, c: true, v: false });
    }

    #[test]
    fn el2_registers_are_out_of_reach_of_el1() {
        let mut bus = Bus::new(uart());
        bus.load(0x80000, &0xd51c1101u32.to_le_bytes()).unwrap(); // msr hcr_el2, x1
        let mut cpu = Cpu::new(0x80000, false);
        cpu.el = 1;
        let register = SystemRegister::HcrEl2;
        assert_eq!(
            cpu.step(&mut bus, false),
            Err(Fault::SystemRegisterAccess { register, el: 1, write: true })
        );
    }

    #[test]
    fn each_el_has_its_own_stack_pointer() {
        let mut cpu = Cpu::new(0, false);
        cpu.write_sp(31, Width::W64, 0x2000);
        cpu.el = 1;
        cpu.write_sp(31, Width::W64, 0x1000);
        cpu.spsel = false;
        assert_eq!(cpu.sp(), 0);
        assert_eq!(cpu.sp_el, [0, 0x1000, 0x2000]);
    }

    #[test]
    fn conditional_select_and_variable_shifts() {
        // movz x1, #3; movz x2, #65; lslv x3, x1, x2 (65 mod 64 = 1);
        // cmp x1, x2; csel x4, x1, x2, lo; tst x1, x2 (3 & 65 = 1, so not zero)
        let program = [0xd2800061, 0xd2800822, 0x9ac22023, 0xeb02003f, 0x9a823024, 0xea02003f];
        let (cpu, _) = run(&program, 6);
        assert_eq!(cpu.x[3], 6);
        assert_eq!(cpu.x[4], 3);
        assert_eq!(cpu.flags, Flags { n: false, z: false, c: false, v: false });
    }

    #[test]
    fn semihosting_exit_reports_the_code() {
        // Block at 0x1000: [ADP_Stopped_ApplicationExit, 7]
        let mut bus = Bus::new(uart());
        bus.write(0x1000, 8, ADP_STOPPED_APPLICATION_EXIT).unwrap();
        bus.write(0x1008, 8, 7).unwrap();
        bus.load(0x80000, &0xd45e0000u32.to_le_bytes()).unwrap();
        let mut cpu = Cpu::new(0x80000, true);
        cpu.x[0] = SYS_EXIT;
        cpu.x[1] = 0x1000;
        assert_eq!(cpu.step(&mut bus, false), Ok(Step::Exit(7)));

        let mut cpu = Cpu::new(0x80000, false);
        assert_eq!(cpu.step(&mut bus, false), Err(Fault::Halt { imm16: 0xf000 }));
    }
}
