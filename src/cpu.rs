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
    /// hardware faults depends on SCTLR_EL2.SA, which Nebu does not model yet.
    StackAlignment { sp: u64 },
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
            Fault::Halt { imm16 } => {
                write!(f, "hlt #{imm16:#x} with semihosting off (the core halts for a debugger)")
            }
            Fault::Semihosting { operation } => {
                write!(f, "semihosting operation {operation:#x} not implemented")
            }
        }
    }
}

/// The state of one core.
pub struct Cpu {
    /// General-purpose registers x0 to x30.
    pub x: [u64; 31],
    /// The stack pointer. The core runs at EL2 with SPSel set, so this is SP_EL2.
    pub sp: u64,
    /// The program counter.
    pub pc: u64,
    /// The condition flags.
    pub flags: Flags,
    /// The current exception level.
    pub el: u8,
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
            sp: 0,
            pc: entry,
            flags: Flags { z: true, ..Flags::default() },
            el: 2,
            event: false,
            semihosting,
        }
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
        if n == 31 { self.sp & width.mask() } else { self.read(n, width) }
    }

    /// Writes register `n`, where 31 is the stack pointer.
    fn write_sp(&mut self, n: u8, width: Width, value: u64) {
        if n == 31 { self.sp = value & width.mask() } else { self.write(n, width, value) }
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
        if rn == 31 && !self.sp.is_multiple_of(16) {
            return Err(Fault::StackAlignment { sp: self.sp });
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
            Movz { width, rd, imm16, shift } => self.write(rd, width, u64::from(imm16) << shift),
            Movk { width, rd, imm16, shift } => {
                let kept = self.read(rd, width) & !(0xFFFF << shift);
                self.write(rd, width, kept | u64::from(imm16) << shift);
            }
            Bfm { width, rd, rn, immr, imms } => {
                let n = u32::from(width == Width::W64);
                let (wmask, tmask) =
                    decode_bit_masks(n, u32::from(imms), u32::from(immr), false, width)
                        .expect("the decoder only accepts valid bitfield encodings");
                let dst = self.read(rd, width);
                let src = self.read(rn, width);
                let rotated = rotate_right(src, u32::from(immr), width);
                let bottom = (dst & !wmask) | (rotated & wmask);
                self.write(rd, width, (dst & !tmask) | (bottom & tmask));
            }
            SubsImmediate { width, rd, rn, imm } => {
                let operand1 = self.read_sp(rn, width);
                let (value, flags) = add_with_carry(operand1, !imm & width.mask(), true, width);
                self.write(rd, width, value);
                self.flags = flags;
            }
            OrrShiftedRegister { width, rd, rn, rm, shift, amount } => {
                let operand2 = shift_register(self.read(rm, width), shift, amount, width);
                self.write(rd, width, self.read(rn, width) | operand2);
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
            Ret { rn } => next = self.read(rn, Width::W64),
            BCond { condition, offset } => {
                if self.condition_holds(condition) {
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
            // No pipeline is modelled, so a barrier has nothing to wait for.
            Nop | Isb => {}
            Wfe => {
                if self.event {
                    self.event = false;
                } else {
                    result = Step::Asleep;
                }
            }
            Mrs { rt, register } => {
                let value = match register {
                    SystemRegister::MpidrEl1 => MPIDR_CORE_0,
                    SystemRegister::CurrentEl => u64::from(self.el) << 2,
                };
                self.write(rt, Width::W64, value);
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
                let offset = match extend {
                    Extend::Uxtw => self.read(rm, Width::W32),
                    Extend::Sxtw => self.read(rm, Width::W32) as u32 as i32 as i64 as u64,
                    Extend::Lsl | Extend::Sxtx => self.read(rm, Width::W64),
                };
                let address = self.base(rn)?.wrapping_add(offset);
                let value = load(bus, address, 1)?;
                self.write(rt, Width::W32, value);
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
        }

        self.pc = next;
        Ok(result)
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
        let mut bus = Bus::new(Pl011::new(Box::new(std::io::sink())));
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
    fn semihosting_exit_reports_the_code() {
        // Block at 0x1000: [ADP_Stopped_ApplicationExit, 7]
        let mut bus = Bus::new(Pl011::new(Box::new(std::io::sink())));
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
