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
use crate::mmu::{self, Access, SCTLR_M};

/// MPIDR_EL1 for core 0, as QEMU reports it: bit 31 is RES1 and the core number is
/// in the low byte.
const MPIDR_CORE_0: u64 = 0x8000_0000;

/// The rate the generic timer's counter counts at: 62.5 MHz, as QEMU's `raspi3b`
/// reports in CNTFRQ_EL0 (ADR 0016).
const COUNTER_FREQUENCY: u64 = 62_500_000;
/// CNTKCTL_EL1's bit that lets EL0 read CNTVCT_EL0 and CNTFRQ_EL0 (ADR 0019).
const CNTKCTL_EL0VCTEN: u64 = 1 << 1;
/// CNTV_CTL_EL0's bits: ENABLE, IMASK, and ISTATUS, which only reads.
const CNTV_CTL_ENABLE: u64 = 1 << 0;
const CNTV_CTL_IMASK: u64 = 1 << 1;
const CNTV_CTL_ISTATUS: u64 = 1 << 2;
/// PSTATE.I, in Nebu's four DAIF bits: IRQs are masked.
const DAIF_I: u8 = 0b0010;

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
    /// model, such as a page size other than 4 KiB.
    Unsupported(&'static str),
    /// Reading a page table entry failed.
    PageTable { address: u64, error: BusError },
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
            Fault::PageTable { address, error } => {
                write!(f, "reading the page table entry at {address:#x} failed: {error}")
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

/// The system registers Nebu models, apart from the read-only identification ones.
#[derive(Clone, Copy, Debug, Default)]
struct SystemRegisters {
    /// SCTLR_EL1, TCR_EL1, MAIR_EL1, TTBR0_EL1 and TTBR1_EL1.
    translation: mmu::Registers,
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
    cpacr_el1: u64,
    cntkctl_el1: u64,
    tpidr_el0: u64,
    cntv_ctl_el0: u64,
    cntv_cval_el0: u64,
}

/// HCR_EL2.RW: EL1 runs AArch64.
const HCR_EL2_RW: u64 = 1 << 31;
/// The SCTLR_EL1 bits Nebu models: the RES1 bits, M (the MMU), C (the data cache),
/// SA and SA0 (stack alignment checks) and I (the instruction cache). Nebu has no
/// caches, and halts on any misaligned SP-based access whether or not SA is set.
const SCTLR_EL1_MODELLED: u64 = 0x30d0_0800 | 1 << 12 | 1 << 4 | 1 << 3 | 1 << 2 | SCTLR_M;
/// The Cortex-A53's cache type register, as QEMU gives it: 64-byte cache lines, and
/// an instruction cache indexed and tagged by physical address.
const CTR_EL0: u64 = 0x8444_8004;

/// Exception classes, the top 6 bits of ESR_ELx.
const EC_UNKNOWN: u64 = 0x00;
const EC_SVC: u64 = 0x15;
const EC_INSTRUCTION_ABORT_LOWER_EL: u64 = 0x20;
const EC_INSTRUCTION_ABORT_SAME_EL: u64 = 0x21;
const EC_DATA_ABORT_LOWER_EL: u64 = 0x24;
const EC_DATA_ABORT_SAME_EL: u64 = 0x25;
const EC_BRK: u64 = 0x3C;
/// ESR_ELx.WnR for a data abort: the access was a write.
const ESR_WNR: u64 = 1 << 6;
/// ESR_ELx.IL: the instruction that caused the exception was 32 bits long.
const ESR_IL: u64 = 1 << 25;

/// Why an instruction stopped before it finished.
enum Stop {
    /// Nebu cannot carry on.
    Fault(Fault),
    /// An instruction fetch or data access failed translation, so the CPU takes an
    /// abort instead of finishing the instruction.
    Abort { access: Access, address: u64, status: u8 },
}

impl From<Fault> for Stop {
    fn from(fault: Fault) -> Self {
        Stop::Fault(fault)
    }
}

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
    /// PSTATE.DAIF: which exceptions are masked. Only I, for the virtual timer's
    /// IRQ, has any effect (ADR 0016).
    daif: u8,
    /// How many instructions the core has run, which is also its physical counter:
    /// one count for each (ADR 0016).
    count: u64,
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
            count: 0,
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
        // Instruction and data aborts from EL0 use the "lower EL" classes, one less
        // than the "same EL" ones.
        let lower = self.el == 0;
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
        let ec = match ec {
            EC_INSTRUCTION_ABORT_SAME_EL if lower => EC_INSTRUCTION_ABORT_LOWER_EL,
            EC_DATA_ABORT_SAME_EL if lower => EC_DATA_ABORT_LOWER_EL,
            ec => ec,
        };
        self.sys.esr_el1 = ec << 26 | ESR_IL | iss;
        self.el = 1;
        self.spsel = true;
        self.daif = 0b1111;
        Ok(self.sys.vbar_el1.wrapping_add(offset))
    }

    /// Takes an IRQ before the instruction at the program counter, which runs when the
    /// handler returns. Returns the address of the vector to run. ESR_EL1 is left as
    /// it is, as the architecture says for interrupts.
    fn take_irq(&mut self) -> Result<u64, Fault> {
        // IRQ vectors are 0x80 after the synchronous ones.
        let offset = match (self.el, self.spsel) {
            (0, _) => 0x480,
            (1, false) => 0x080,
            (1, true) => 0x280,
            _ => return Err(Fault::Unsupported("taking an IRQ at EL2")),
        };
        self.sys.spsr_el1 = self.pstate();
        self.sys.elr_el1 = self.pc;
        self.el = 1;
        self.spsel = true;
        self.daif = 0b1111;
        Ok(self.sys.vbar_el1.wrapping_add(offset))
    }

    /// The virtual counter: the physical one, less CNTVOFF_EL2.
    fn virtual_count(&self) -> u64 {
        self.count.wrapping_sub(self.sys.cntvoff_el2)
    }

    /// Whether the virtual timer's condition is met: it is enabled, and the virtual
    /// counter has reached its comparator. This is CNTV_CTL_EL0.ISTATUS.
    fn virtual_timer_met(&self) -> bool {
        self.sys.cntv_ctl_el0 & CNTV_CTL_ENABLE != 0
            && self.virtual_count() >= self.sys.cntv_cval_el0
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
    fn minimum_el(&self, register: SystemRegister) -> u8 {
        use SystemRegister::*;
        match register {
            // EL0 may read the counter and its frequency when CNTKCTL_EL1.EL0VCTEN is
            // set (ADR 0019).
            CntfrqEl0 | CntvctEl0 if self.sys.cntkctl_el1 & CNTKCTL_EL0VCTEN != 0 => 0,
            MpidrEl1 | CurrentEl | SctlrEl1 | SpsrEl1 | ElrEl1 | SpEl0 | EsrEl1 | FarEl1
            | VbarEl1 | TcrEl1 | MairEl1 | Ttbr0El1 | Ttbr1El1 | CpacrEl1 => 1,
            // EL0 can read it only when SCTLR_EL1.UCT is set, which Nebu does not allow.
            CtrEl0 => 1,
            TpidrEl0 => 0,
            // EL0 can reach them only as CNTKCTL_EL1 allows, and Nebu allows only the
            // counter and its frequency.
            CntfrqEl0 | CntvctEl0 | CntvCtlEl0 | CntvCvalEl0 => 1,
            CntkctlEl1 => 1,
            HcrEl2 | CnthctlEl2 | CntvoffEl2 | SpsrEl2 | ElrEl2 | SpEl1 => 2,
        }
    }

    /// MRS: reads a system register.
    fn read_system_register(&self, register: SystemRegister) -> Result<u64, Fault> {
        if self.el < self.minimum_el(register) {
            return Err(Fault::SystemRegisterAccess { register, el: self.el, write: false });
        }
        use SystemRegister::*;
        Ok(match register {
            MpidrEl1 => MPIDR_CORE_0,
            CurrentEl => u64::from(self.el) << 2,
            SctlrEl1 => self.sys.translation.sctlr,
            TcrEl1 => self.sys.translation.tcr,
            MairEl1 => self.sys.translation.mair,
            Ttbr0El1 => self.sys.translation.ttbr0,
            Ttbr1El1 => self.sys.translation.ttbr1,
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
            CpacrEl1 => self.sys.cpacr_el1,
            CtrEl0 => CTR_EL0,
            TpidrEl0 => self.sys.tpidr_el0,
            CntfrqEl0 => COUNTER_FREQUENCY,
            CntvctEl0 => self.virtual_count(),
            CntvCtlEl0 => {
                let status = if self.virtual_timer_met() { CNTV_CTL_ISTATUS } else { 0 };
                self.sys.cntv_ctl_el0 | status
            }
            CntvCvalEl0 => self.sys.cntv_cval_el0,
            CntkctlEl1 => self.sys.cntkctl_el1,
        })
    }

    /// MSR (register): writes a system register.
    fn write_system_register(&mut self, register: SystemRegister, value: u64) -> Result<(), Fault> {
        if self.el < self.minimum_el(register) {
            return Err(Fault::SystemRegisterAccess { register, el: self.el, write: true });
        }
        use SystemRegister::*;
        match register {
            MpidrEl1 | CurrentEl | EsrEl1 | FarEl1 | CtrEl0 | CntvctEl0 => {
                // Writable on hardware in the case of ESR and FAR, but nothing should.
                return Err(Fault::SystemRegisterAccess { register, el: self.el, write: true });
            }
            SctlrEl1 => {
                if value & !SCTLR_EL1_MODELLED != 0 {
                    return Err(Fault::Unsupported(
                        "SCTLR_EL1 bits other than M, C, SA, SA0 and I",
                    ));
                }
                self.sys.translation.sctlr = value;
            }
            // Checked when they are used for a translation.
            TcrEl1 => self.sys.translation.tcr = value,
            MairEl1 => self.sys.translation.mair = value,
            Ttbr0El1 => self.sys.translation.ttbr0 = value,
            Ttbr1El1 => self.sys.translation.ttbr1 = value,
            SpsrEl1 => self.sys.spsr_el1 = value,
            ElrEl1 => self.sys.elr_el1 = value,
            SpEl0 => self.sp_el[0] = value,
            TpidrEl0 => self.sys.tpidr_el0 = value,
            // The low 11 bits are RES0: the table is 2 KiB-aligned.
            VbarEl1 => self.sys.vbar_el1 = value & !0x7FF,
            HcrEl2 => {
                if value & !HCR_EL2_RW != 0 {
                    return Err(Fault::Unsupported("HCR_EL2 bits other than RW"));
                }
                self.sys.hcr_el2 = value;
            }
            // Nebu lets EL1 use the counter and timers whatever this says.
            CnthctlEl2 => self.sys.cnthctl_el2 = value,
            CntvoffEl2 => self.sys.cntvoff_el2 = value,
            SpsrEl2 => self.sys.spsr_el2 = value,
            ElrEl2 => self.sys.elr_el2 = value,
            SpEl1 => self.sp_el[1] = value,
            // Nebu has no floating-point or SIMD instructions, so they halt it
            // whatever FPEN (bits 21 and 20) says. Only FPEN = 0b00, which makes them
            // trap, matches that.
            CpacrEl1 => {
                if value != 0 {
                    return Err(Fault::Unsupported("CPACR_EL1 values that allow FP or SIMD"));
                }
                self.sys.cpacr_el1 = value;
            }
            // Only the firmware sets the rate, before the kernel starts.
            CntfrqEl0 => return Err(Fault::Unsupported("writing CNTFRQ_EL0")),
            // ISTATUS only reads, so a write keeps only ENABLE and IMASK.
            CntvCtlEl0 => self.sys.cntv_ctl_el0 = value & (CNTV_CTL_ENABLE | CNTV_CTL_IMASK),
            CntvCvalEl0 => self.sys.cntv_cval_el0 = value,
            // EL0 is kept away from the timers, and no events are raised from the
            // counter (ADR 0016). It may read the virtual counter and its frequency
            // (ADR 0019).
            CntkctlEl1 => {
                if value & !CNTKCTL_EL0VCTEN != 0 {
                    return Err(Fault::Unsupported(
                        "CNTKCTL_EL1 values that give EL0 more than the virtual counter",
                    ));
                }
                self.sys.cntkctl_el1 = value;
            }
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

    /// Translates `address` for `access` at the current exception level.
    fn translate(
        &self,
        bus: &mut Bus,
        address: u64,
        access: Access,
    ) -> Result<mmu::Translation, Stop> {
        mmu::translate(bus, &self.sys.translation, address, access, self.el).map_err(|error| {
            match error {
                mmu::Error::Abort { status } => Stop::Abort { access, address, status },
                mmu::Error::Table { address, error } => {
                    Stop::Fault(Fault::PageTable { address, error })
                }
                mmu::Error::Unsupported(what) => Stop::Fault(Fault::Unsupported(what)),
            }
        })
    }

    /// Reads `size` bytes at virtual address `address`.
    fn load(&self, bus: &mut Bus, address: u64, size: u64) -> Result<u64, Stop> {
        let translation = self.translate(bus, address, Access::Read)?;
        let physical = translation.address;
        if address.is_multiple_of(size) || translation.device {
            // A misaligned access to Device memory is an error from the bus.
            return Ok(bus.read(physical, size).map_err(|error| Fault::Memory {
                address,
                write: false,
                error,
            })?);
        }
        // Normal memory allows misaligned accesses: do it a byte at a time.
        if physical % 4096 + size > 4096 {
            return Err(Fault::Unsupported("a misaligned access that crosses a page").into());
        }
        let mut value = 0;
        for index in 0..size {
            let byte = bus.read(physical + index, 1).map_err(|error| Fault::Memory {
                address,
                write: false,
                error,
            })?;
            value |= byte << (8 * index);
        }
        Ok(value)
    }

    /// Writes the low `size` bytes of `value` at virtual address `address`.
    fn store(&self, bus: &mut Bus, address: u64, size: u64, value: u64) -> Result<(), Stop> {
        let translation = self.translate(bus, address, Access::Write)?;
        let physical = translation.address;
        let fault = |error| Fault::Memory { address, write: true, error };
        if address.is_multiple_of(size) || translation.device {
            return Ok(bus.write(physical, size, value).map_err(fault)?);
        }
        if physical % 4096 + size > 4096 {
            return Err(Fault::Unsupported("a misaligned access that crosses a page").into());
        }
        for index in 0..size {
            bus.write(physical + index, 1, value >> (8 * index)).map_err(fault)?;
        }
        Ok(())
    }

    /// Computes the base address of a load or store whose base register is `rn`.
    fn base(&self, rn: u8) -> Result<u64, Fault> {
        if rn == 31 && !self.sp().is_multiple_of(16) {
            return Err(Fault::StackAlignment { sp: self.sp() });
        }
        Ok(self.read_sp(rn, Width::W64))
    }

    /// Fetches, decodes and executes one instruction. With `trace`, prints it first.
    ///
    /// If the instruction's fetch or one of its data accesses fails translation, the
    /// CPU takes an abort instead, and the next instruction is the abort handler's.
    pub fn step(&mut self, bus: &mut Bus, trace: bool) -> Result<Step, Fault> {
        self.count = self.count.wrapping_add(1);
        bus.advance(self.count);
        // The virtual timer's interrupt reaches the core through the local
        // interrupt controller, and is taken between instructions when PSTATE.I is
        // clear (ADR 0016).
        let firing = self.virtual_timer_met() && self.sys.cntv_ctl_el0 & CNTV_CTL_IMASK == 0;
        bus.set_virtual_timer(firing);
        if bus.irq_pending() && self.daif & DAIF_I == 0 {
            if trace {
                eprintln!("irq at {:#010x}", self.pc);
            }
            self.pc = self.take_irq()?;
            return Ok(Step::Continue);
        }
        match self.execute(bus, trace) {
            Ok(step) => Ok(step),
            Err(Stop::Fault(fault)) => Err(fault),
            Err(Stop::Abort { access, address, status }) => {
                if trace {
                    eprintln!("abort: {access:?} of {address:#x}, fault status {status:#04x}");
                }
                let (ec, iss) = match access {
                    Access::Fetch => (EC_INSTRUCTION_ABORT_SAME_EL, u64::from(status)),
                    Access::Read => (EC_DATA_ABORT_SAME_EL, u64::from(status)),
                    Access::Write => (EC_DATA_ABORT_SAME_EL, ESR_WNR | u64::from(status)),
                };
                // The interrupted instruction is retried when the handler returns.
                self.pc = self.take_exception(ec, iss, self.pc)?;
                self.sys.far_el1 = address;
                Ok(Step::Continue)
            }
        }
    }

    fn execute(&mut self, bus: &mut Bus, trace: bool) -> Result<Step, Stop> {
        let pc = self.pc;
        let translation = self.translate(bus, pc, Access::Fetch)?;
        let word = bus.fetch(translation.address).map_err(|error| Fault::Memory {
            address: pc,
            write: false,
            error,
        })?;
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
            Adrp { rd, offset } => {
                self.write(rd, Width::W64, (pc & !0xFFF).wrapping_add_signed(offset));
            }
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
            EorImmediate { width, rd, rn, imm } => {
                let value = self.read(rn, width) ^ imm;
                self.write_sp(rd, width, value);
            }
            AndsImmediate { width, rd, rn, imm } => {
                let value = self.read(rn, width) & imm;
                self.write(rd, width, value);
                let top = width.bits() - 1;
                self.flags = Flags { n: value >> top & 1 == 1, z: value == 0, c: false, v: false };
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
            OrnShiftedRegister { width, rd, rn, rm, shift, amount } => {
                let operand2 = shift_register(self.read(rm, width), shift, amount, width);
                self.write(rd, width, self.read(rn, width) | (!operand2 & width.mask()));
            }
            EorShiftedRegister { width, rd, rn, rm, shift, amount } => {
                let operand2 = shift_register(self.read(rm, width), shift, amount, width);
                self.write(rd, width, self.read(rn, width) ^ operand2);
            }
            BicShiftedRegister { width, rd, rn, rm, shift, amount } => {
                let operand2 = shift_register(self.read(rm, width), shift, amount, width);
                self.write(rd, width, self.read(rn, width) & !operand2);
            }
            AndsShiftedRegister { width, rd, rn, rm, shift, amount }
            | BicsShiftedRegister { width, rd, rn, rm, shift, amount } => {
                let mut operand2 = shift_register(self.read(rm, width), shift, amount, width);
                if matches!(instruction, BicsShiftedRegister { .. }) {
                    operand2 = !operand2 & width.mask();
                }
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
            Udiv { width, rd, rn, rm } => {
                // Division by zero gives zero, and raises nothing.
                let quotient = self.read(rn, width).checked_div(self.read(rm, width));
                self.write(rd, width, quotient.unwrap_or(0));
            }
            Csel { width, rd, rn, rm, condition } => {
                let source = if self.condition_holds(condition) { rn } else { rm };
                self.write(rd, width, self.read(source, width));
            }
            Madd { width, rd, rn, rm, ra } => {
                let product = self.read(rn, width).wrapping_mul(self.read(rm, width));
                self.write(rd, width, self.read(ra, width).wrapping_add(product));
            }
            Umulh { rd, rn, rm } => {
                let product =
                    u128::from(self.read(rn, Width::W64)) * u128::from(self.read(rm, Width::W64));
                self.write(rd, Width::W64, (product >> 64) as u64);
            }
            Msub { width, rd, rn, rm, ra } => {
                let product = self.read(rn, width).wrapping_mul(self.read(rm, width));
                self.write(rd, width, self.read(ra, width).wrapping_sub(product));
            }
            Csinc { width, rd, rn, rm, condition } => {
                let value = if self.condition_holds(condition) {
                    self.read(rn, width)
                } else {
                    self.read(rm, width).wrapping_add(1)
                };
                self.write(rd, width, value);
            }
            Csinv { width, rd, rn, rm, condition } => {
                let value = if self.condition_holds(condition) {
                    self.read(rn, width)
                } else {
                    !self.read(rm, width) & width.mask()
                };
                self.write(rd, width, value);
            }
            CcmpRegister { width, rn, rm, nzcv, condition } => {
                let operand2 = self.read(rm, width);
                self.conditional_compare(width, rn, operand2, nzcv, condition);
            }
            CcmpImmediate { width, rn, imm, nzcv, condition } => {
                self.conditional_compare(width, rn, u64::from(imm), nzcv, condition);
            }
            CcmnImmediate { width, rn, imm, nzcv, condition } => {
                self.conditional_compare_negative(width, rn, u64::from(imm), nzcv, condition);
            }
            SubShiftedRegister { width, rd, rn, rm, shift, amount } => {
                let operand2 = shift_register(self.read(rm, width), shift, amount, width);
                self.write(rd, width, self.read(rn, width).wrapping_sub(operand2));
            }
            AddShiftedRegister { width, rd, rn, rm, shift, amount } => {
                let operand2 = shift_register(self.read(rm, width), shift, amount, width);
                self.write(rd, width, self.read(rn, width).wrapping_add(operand2));
            }
            AddsShiftedRegister { width, rd, rn, rm, shift, amount } => {
                let operand1 = self.read(rn, width);
                let operand2 = shift_register(self.read(rm, width), shift, amount, width);
                let (value, flags) = add_with_carry(operand1, operand2, false, width);
                self.write(rd, width, value);
                self.flags = flags;
            }
            Rbit { width, rd, rn } => {
                let value = self.read(rn, width).reverse_bits() >> (64 - width.bits());
                self.write(rd, width, value);
            }
            Clz { width, rd, rn } => {
                let value = self.read(rn, width).leading_zeros() - (64 - width.bits());
                self.write(rd, width, u64::from(value));
            }
            Extr { width, rd, rn, rm, lsb } => {
                let (high, low) = (self.read(rn, width), self.read(rm, width));
                let value = if lsb == 0 {
                    low
                } else {
                    low >> lsb | high << (width.bits() - u32::from(lsb))
                };
                self.write(rd, width, value);
            }
            AddExtendedRegister { width, rd, rn, rm, option, amount } => {
                let operand2 = self.extended(rm, option, amount, width);
                let value = self.read_sp(rn, width).wrapping_add(operand2) & width.mask();
                self.write_sp(rd, width, value);
            }
            SubsExtendedRegister { width, rd, rn, rm, option, amount } => {
                let operand2 = self.extended(rm, option, amount, width);
                let (value, flags) =
                    add_with_carry(self.read_sp(rn, width), !operand2 & width.mask(), true, width);
                self.write(rd, width, value);
                self.flags = flags;
            }
            Sbc { width, rd, rn, rm } => {
                let operand2 = !self.read(rm, width) & width.mask();
                let (value, _) =
                    add_with_carry(self.read(rn, width), operand2, self.flags.c, width);
                self.write(rd, width, value);
            }
            Sbcs { width, rd, rn, rm } => {
                let operand2 = !self.read(rm, width) & width.mask();
                let (value, flags) =
                    add_with_carry(self.read(rn, width), operand2, self.flags.c, width);
                self.write(rd, width, value);
                self.flags = flags;
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
            Blr { rn } => {
                next = self.read(rn, Width::W64);
                self.write(30, Width::W64, pc.wrapping_add(4));
            }
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
                    return Err(Fault::Halt { imm16 }.into());
                }
                result = self.semihosting_call(bus)?;
            }
            Brk { imm16 } => next = self.take_exception(EC_BRK, u64::from(imm16), pc)?,
            Udf { .. } => next = self.take_exception(EC_UNKNOWN, 0, pc)?,
            // A system call returns to the instruction after it.
            Svc { imm16 } => {
                next = self.take_exception(EC_SVC, u64::from(imm16), pc.wrapping_add(4))?
            }
            Eret => next = self.exception_return()?,
            // No pipeline, caches or TLB are modelled, so a barrier has nothing to wait
            // for and there is nothing to invalidate.
            Nop | Isb | Dsb { .. } => {}
            // These are undefined at EL0, as on hardware.
            IcIallu | TlbiVmalle1 | TlbiAside1 { .. } if self.el == 0 => {
                next = self.take_exception(EC_UNKNOWN, 0, pc)?
            }
            IcIallu | TlbiVmalle1 | TlbiAside1 { .. } => {}
            // Nor are data caches, so there is nothing to clean, but the address must
            // still translate, as on hardware.
            DcCivac { rt } => {
                if self.el == 0 {
                    return Err(Fault::Unsupported("cache maintenance at EL0").into());
                }
                self.translate(bus, self.read(rt, Width::W64), Access::Read)?;
            }
            // WFI waits for an interrupt to be pending, even a masked one. Nebu skips
            // straight to the next one due (ADR 0017), or waits for input (ADR 0018).
            Wfi if self.el == 0 => {
                return Err(Fault::Unsupported("wfi at EL0").into());
            }
            Wfi if bus.irq_pending() => {}
            Wfi => {
                let timer = (self.sys.cntv_ctl_el0 & (CNTV_CTL_ENABLE | CNTV_CTL_IMASK)
                    == CNTV_CTL_ENABLE
                    && bus.virtual_timer_routed())
                .then(|| self.sys.cntv_cval_el0.saturating_add(self.sys.cntvoff_el2));
                match [timer, bus.next_device_interrupt()].into_iter().flatten().min() {
                    // The next step counts one more, and finds the interrupt raised.
                    // A deadline that has passed is already due.
                    Some(due) => self.count = self.count.max(due.saturating_sub(1)),
                    // With nothing due, only input to the UART can wake the core
                    // (ADR 0018).
                    None if bus.wait_for_input() => {}
                    None => result = Step::Asleep,
                }
            }
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
                let value = self.load(bus, branch(offset), width.bytes())?;
                self.write(rt, width, value);
            }
            LdrImmediate { width, rt, rn, index, offset } => {
                let (address, writeback) = self.address(rn, index, offset)?;
                let value = self.load(bus, address, width.bytes())?;
                if let Some(base) = writeback {
                    self.write_sp(rn, Width::W64, base);
                }
                self.write(rt, width, value);
            }
            StrImmediate { width, rt, rn, index, offset } => {
                let (address, writeback) = self.address(rn, index, offset)?;
                self.store(bus, address, width.bytes(), self.read(rt, width))?;
                if let Some(base) = writeback {
                    self.write_sp(rn, Width::W64, base);
                }
            }
            LdrbImmediate { rt, rn, index, offset } => {
                let (address, writeback) = self.address(rn, index, offset)?;
                let value = self.load(bus, address, 1)?;
                if let Some(base) = writeback {
                    self.write_sp(rn, Width::W64, base);
                }
                self.write(rt, Width::W32, value);
            }
            LdrsbImmediate { width, rt, rn, index, offset } => {
                let (address, writeback) = self.address(rn, index, offset)?;
                let value = self.load(bus, address, 1)?;
                if let Some(base) = writeback {
                    self.write_sp(rn, Width::W64, base);
                }
                self.write(rt, width, value as u8 as i8 as i64 as u64);
            }
            LdrbRegister { rt, rn, rm, extend } => {
                let address = self.base(rn)?.wrapping_add(self.extend_register(rm, extend));
                let value = self.load(bus, address, 1)?;
                self.write(rt, Width::W32, value);
            }
            StrbRegister { rt, rn, rm, extend } => {
                let address = self.base(rn)?.wrapping_add(self.extend_register(rm, extend));
                self.store(bus, address, 1, self.read(rt, Width::W32))?;
            }
            LdrsbRegister { width, rt, rn, rm, extend } => {
                let address = self.base(rn)?.wrapping_add(self.extend_register(rm, extend));
                let value = self.load(bus, address, 1)?;
                self.write(rt, width, value as u8 as i8 as i64 as u64);
            }
            LdrRegister { width, rt, rn, rm, extend, amount } => {
                let offset = self.extend_register(rm, extend) << amount;
                let address = self.base(rn)?.wrapping_add(offset);
                let value = self.load(bus, address, width.bytes())?;
                self.write(rt, width, value);
            }
            LdrhRegister { rt, rn, rm, extend, amount } => {
                let offset = self.extend_register(rm, extend) << amount;
                let address = self.base(rn)?.wrapping_add(offset);
                let value = self.load(bus, address, 2)?;
                self.write(rt, Width::W32, value);
            }
            LdrswRegister { rt, rn, rm, extend, amount } => {
                let offset = self.extend_register(rm, extend) << amount;
                let address = self.base(rn)?.wrapping_add(offset);
                let value = self.load(bus, address, 4)?;
                self.write(rt, Width::W64, value as u32 as i32 as i64 as u64);
            }
            StrRegister { width, rt, rn, rm, extend, amount } => {
                let offset = self.extend_register(rm, extend) << amount;
                let address = self.base(rn)?.wrapping_add(offset);
                self.store(bus, address, width.bytes(), self.read(rt, width))?;
            }
            StrbImmediate { rt, rn, index, offset } => {
                let (address, writeback) = self.address(rn, index, offset)?;
                self.store(bus, address, 1, self.read(rt, Width::W32))?;
                if let Some(base) = writeback {
                    self.write_sp(rn, Width::W64, base);
                }
            }
            LdrhImmediate { rt, rn, index, offset } => {
                let (address, writeback) = self.address(rn, index, offset)?;
                let value = self.load(bus, address, 2)?;
                if let Some(base) = writeback {
                    self.write_sp(rn, Width::W64, base);
                }
                self.write(rt, Width::W32, value);
            }
            StrhImmediate { rt, rn, index, offset } => {
                let (address, writeback) = self.address(rn, index, offset)?;
                self.store(bus, address, 2, self.read(rt, Width::W32))?;
                if let Some(base) = writeback {
                    self.write_sp(rn, Width::W64, base);
                }
            }
            Stur { width, rt, rn, offset } => {
                let (address, _) = self.address(rn, Index::Offset, offset)?;
                self.store(bus, address, width.bytes(), self.read(rt, width))?;
            }
            Stp { width, rt, rt2, rn, index, offset } => {
                let (address, writeback) = self.address(rn, index, offset)?;
                self.store(bus, address, width.bytes(), self.read(rt, width))?;
                self.store(
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
                let first = self.load(bus, address, width.bytes())?;
                let second = self.load(bus, address.wrapping_add(width.bytes()), width.bytes())?;
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

    /// CCMP: if `condition` holds, sets the flags as comparing `rn` with `operand2`
    /// would, and otherwise to `nzcv`.
    fn conditional_compare(
        &mut self,
        width: Width,
        rn: u8,
        operand2: u64,
        nzcv: u8,
        condition: u8,
    ) {
        self.flags = if self.condition_holds(condition) {
            add_with_carry(self.read(rn, width), !operand2 & width.mask(), true, width).1
        } else {
            Self::nzcv_flags(nzcv)
        };
    }

    /// CCMN: like [`Self::conditional_compare`], but comparing with `-operand2`.
    fn conditional_compare_negative(
        &mut self,
        width: Width,
        rn: u8,
        operand2: u64,
        nzcv: u8,
        condition: u8,
    ) {
        self.flags = if self.condition_holds(condition) {
            add_with_carry(self.read(rn, width), operand2, false, width).1
        } else {
            Self::nzcv_flags(nzcv)
        };
    }

    /// The flags an `nzcv` field names.
    fn nzcv_flags(nzcv: u8) -> Flags {
        Flags {
            n: nzcv & 0b1000 != 0,
            z: nzcv & 0b0100 != 0,
            c: nzcv & 0b0010 != 0,
            v: nzcv & 0b0001 != 0,
        }
    }

    /// The second operand of ADD and SUBS (extended register): `rm` extended by
    /// `option`, shifted left by `amount`.
    fn extended(&self, rm: u8, option: u8, amount: u8, width: Width) -> u64 {
        let value = self.read(rm, Width::W64);
        // UXTB, UXTH, UXTW and UXTX keep the low 8, 16, 32 or 64 bits; the SXT forms
        // sign-extend them.
        let bits = 8u32 << (option & 0b11);
        let extended = if bits == 64 {
            value
        } else if option & 0b100 == 0 {
            value & ((1 << bits) - 1)
        } else {
            ((value << (64 - bits)) as i64 >> (64 - bits)) as u64
        };
        (extended << amount) & width.mask()
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
    fn semihosting_call(&mut self, bus: &mut Bus) -> Result<Step, Stop> {
        // The operation number is in w0.
        let operation = self.read(0, Width::W32);
        if operation != SYS_EXIT {
            return Err(Fault::Semihosting { operation }.into());
        }
        // On AArch64, x1 points to a block holding the reason and the exit code.
        let block = self.read(1, Width::W64);
        let reason = self.load(bus, block, 8)?;
        let code = self.load(bus, block.wrapping_add(8), 8)?;
        // Like QEMU, any reason other than a normal exit counts as failure.
        let code = if reason == ADP_STOPPED_APPLICATION_EXIT { code as i32 } else { 1 };
        Ok(Step::Exit(code))
    }
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
    fn sbc_subtracts_the_borrow_and_keeps_the_flags() {
        // sbc x16, x9, xzr: x9 - 0 - (1 - C).
        for (carry, expected) in [(true, 5), (false, 4)] {
            let mut bus = Bus::new(uart());
            bus.load(0x80000, &0xda1f0130u32.to_le_bytes()).unwrap();
            let mut cpu = Cpu::new(0x80000, false);
            cpu.x[9] = 5;
            cpu.flags = Flags { n: true, z: false, c: carry, v: false };
            cpu.step(&mut bus, false).unwrap();
            assert_eq!(cpu.x[16], expected);
            assert_eq!(cpu.flags, Flags { n: true, z: false, c: carry, v: false });
        }
        // sbc w0, w1, w2: 32 bits wrap, and the upper half is cleared.
        let mut bus = Bus::new(uart());
        bus.load(0x80000, &0x5a020020u32.to_le_bytes()).unwrap();
        let mut cpu = Cpu::new(0x80000, false);
        (cpu.x[1], cpu.x[2]) = (0xFFFF_FFFF_0000_0001, 3);
        cpu.step(&mut bus, false).unwrap();
        assert_eq!(cpu.x[0], 0xFFFF_FFFD);
    }

    #[test]
    fn ldrsw_sign_extends_the_word_it_loads() {
        // ldrsw x12, [x10, x0, lsl #2], loading the second word of a table.
        for (word, expected) in [(0xFFFF_FFFEu32, 0xFFFF_FFFF_FFFF_FFFE), (7, 7)] {
            let mut bus = Bus::new(uart());
            bus.load(0x80000, &0xb8a0794cu32.to_le_bytes()).unwrap();
            bus.load(0x90004, &word.to_le_bytes()).unwrap();
            let mut cpu = Cpu::new(0x80000, false);
            (cpu.x[10], cpu.x[0]) = (0x90000, 1);
            cpu.step(&mut bus, false).unwrap();
            assert_eq!(cpu.x[12], expected);
        }
    }

    #[test]
    fn ldrsb_sign_extends_the_byte_it_loads() {
        // ldrsb w13, [x0, x9] and ldrsb x13, [x0, x9].
        for (instruction, expected) in [(0x38e9680du32, 0xFFFF_FF80), (0x38a9680d, u64::MAX - 0x7F)]
        {
            let mut bus = Bus::new(uart());
            bus.load(0x80000, &instruction.to_le_bytes()).unwrap();
            bus.load(0x90003, &[0x80]).unwrap();
            let mut cpu = Cpu::new(0x80000, false);
            (cpu.x[0], cpu.x[9], cpu.x[13]) = (0x90000, 3, u64::MAX);
            cpu.step(&mut bus, false).unwrap();
            assert_eq!(cpu.x[13], expected);
        }
    }

    #[test]
    fn strb_register_stores_the_low_byte() {
        // strb w10, [x24, x11].
        let mut bus = Bus::new(uart());
        bus.load(0x80000, &0x382b6b0au32.to_le_bytes()).unwrap();
        let mut cpu = Cpu::new(0x80000, false);
        (cpu.x[10], cpu.x[24], cpu.x[11]) = (0x1234, 0x90000, 5);
        cpu.step(&mut bus, false).unwrap();
        assert_eq!(bus.read(0x90005, 1).unwrap(), 0x34);
        assert_eq!(bus.read(0x90006, 1).unwrap(), 0);
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
    fn svc_from_el0_goes_to_the_lower_el_vector() {
        let mut bus = Bus::new(uart());
        bus.load(0x80000, &0xd4000021u32.to_le_bytes()).unwrap(); // svc #1
        let mut cpu = Cpu::new(0x80000, false);
        cpu.el = 0;
        cpu.spsel = false;
        cpu.daif = 0b1111;
        cpu.sys.vbar_el1 = 0x1000;
        cpu.step(&mut bus, false).unwrap();
        // EC 0x15 with IL and the immediate; it returns to the next instruction.
        assert_eq!(cpu.sys.esr_el1, 0x5600_0001);
        assert_eq!(cpu.sys.elr_el1, 0x80004);
        assert_eq!(cpu.sys.spsr_el1, 0x3c0 | 1 << 30); // EL0t, masked, Z set
        assert_eq!((cpu.el, cpu.spsel, cpu.pc), (1, true, 0x1400));
    }

    #[test]
    fn tlb_maintenance_is_undefined_at_el0() {
        let mut bus = Bus::new(uart());
        bus.load(0x80000, &0xd5088748u32.to_le_bytes()).unwrap(); // tlbi aside1, x8
        let mut cpu = Cpu::new(0x80000, false);
        cpu.el = 0;
        cpu.spsel = false;
        cpu.sys.vbar_el1 = 0x1000;
        cpu.step(&mut bus, false).unwrap();
        // EC 0 (unknown reason), returning to the instruction itself.
        assert_eq!(cpu.sys.esr_el1 >> 26, 0);
        assert_eq!(cpu.sys.elr_el1, 0x80000);
        assert_eq!((cpu.el, cpu.pc), (1, 0x1400));
    }

    #[test]
    fn el0_can_use_its_thread_pointer() {
        let mut bus = Bus::new(uart());
        bus.load(0x80000, &0xd51bd048u32.to_le_bytes()).unwrap(); // msr tpidr_el0, x8
        bus.load(0x80004, &0xd53bd049u32.to_le_bytes()).unwrap(); // mrs x9, tpidr_el0
        let mut cpu = Cpu::new(0x80000, false);
        cpu.el = 0;
        cpu.spsel = false;
        cpu.write(8, Width::W64, 0x1234);
        cpu.step(&mut bus, false).unwrap();
        cpu.step(&mut bus, false).unwrap();
        assert_eq!(cpu.read(9, Width::W64), 0x1234);
        assert_eq!(cpu.el, 0);
    }

    #[test]
    fn el0_reads_the_counter_only_when_cntkctl_allows() {
        let mut bus = Bus::new(uart());
        bus.load(0x80000, &0xd518e101u32.to_le_bytes()).unwrap(); // msr cntkctl_el1, x1
        bus.load(0x80004, &0xd53be042u32.to_le_bytes()).unwrap(); // mrs x2, cntvct_el0
        let mut cpu = Cpu::new(0x80004, false);
        cpu.el = 0;
        cpu.spsel = false;
        let register = SystemRegister::CntvctEl0;
        assert_eq!(
            cpu.step(&mut bus, false),
            Err(Fault::SystemRegisterAccess { register, el: 0, write: false })
        );
        cpu.pc = 0x80000;
        cpu.el = 1;
        cpu.write(1, Width::W64, CNTKCTL_EL0VCTEN);
        cpu.step(&mut bus, false).unwrap();
        cpu.el = 0;
        cpu.step(&mut bus, false).unwrap();
        assert!(cpu.read(2, Width::W64) > 0);
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
    fn translation_faults_become_data_aborts() {
        // With the MMU on and TTBR0 switched off: movz x1, #0x1000; str xzr, [x1].
        let mut bus = Bus::new(uart());
        let program: [u32; 2] = [0xd2820001, 0xf900003f];
        let bytes: Vec<u8> = program.iter().flat_map(|word| word.to_le_bytes()).collect();
        bus.load(0x80000, &bytes).unwrap();
        // One level 1 table at 0x10000, mapping the first 1 GiB as a block of normal,
        // accessed, kernel-writable memory at the top of the address space.
        bus.write(0x10000, 8, 0x705).unwrap();
        let mut cpu = Cpu::new(0xFFFF_FF80_0008_0000, false);
        cpu.el = 1;
        cpu.sys.vbar_el1 = 0xFFFF_FF80_0000_0800;
        cpu.sys.translation = mmu::Registers {
            sctlr: SCTLR_M,
            tcr: 0b10 << 30 | 25 << 16 | 1 << 7 | 25,
            mair: 0xFF04,
            ttbr0: 0,
            ttbr1: 0x10000,
        };
        cpu.step(&mut bus, false).unwrap();
        cpu.step(&mut bus, false).unwrap();
        // A level 0 translation fault on a write, from the kernel, at the store.
        assert_eq!(cpu.sys.esr_el1, 0x9600_0044);
        assert_eq!(cpu.sys.far_el1, 0x1000);
        assert_eq!(cpu.sys.elr_el1, 0xFFFF_FF80_0008_0004);
        assert_eq!(cpu.pc, 0xFFFF_FF80_0000_0A00);
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
