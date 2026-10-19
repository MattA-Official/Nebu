//! Nebuchadnezzar (Nebu): a minimal Raspberry Pi 3B emulator.
//!
//! Nebu emulates only the hardware that NeOS uses, and halts loudly on anything else.
//! See `docs/decisions/` for the reasoning behind its design.

#![forbid(unsafe_code)]

mod bus;
mod cpu;
mod decode;
mod gpio;
mod mmu;
mod pl011;

use std::io::Read;
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver};
use std::{env, fs, thread};

use bus::Bus;
use cpu::{Cpu, Step};
use pl011::Pl011;

/// Where the firmware loads `kernel8.img` and starts running it.
const LOAD_ADDRESS: u64 = 0x80000;

/// Exit code when Nebu meets something it cannot emulate.
const FAULT_EXIT_CODE: u8 = 70;

const USAGE: &str = "\
usage: nebu [--semihosting] [--trace] <kernel8.img>

  --semihosting  let `hlt #0xf000` make semihosting calls, as QEMU's -semihosting does
  --trace        print every instruction to standard error before running it";

fn main() -> ExitCode {
    let mut semihosting = false;
    let mut trace = false;
    let mut image = None;
    for argument in env::args().skip(1) {
        match argument.as_str() {
            "--semihosting" => semihosting = true,
            "--trace" => trace = true,
            _ if argument.starts_with('-') || image.is_some() => {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
            _ => image = Some(argument),
        }
    }
    let Some(image) = image else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };

    let bytes = match fs::read(&image) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("nebu: cannot read {image}: {error}");
            return ExitCode::from(2);
        }
    };

    let mut bus = Bus::new(Pl011::new(Box::new(std::io::stdout()), read_stdin()));
    if let Err(error) = bus.load(LOAD_ADDRESS, &bytes) {
        eprintln!("nebu: cannot load {image}: {error}");
        return ExitCode::from(2);
    }
    let mut cpu = Cpu::new(LOAD_ADDRESS, semihosting);

    loop {
        match cpu.step(&mut bus, trace) {
            Ok(Step::Continue) => {}
            Ok(Step::Exit(code)) => return ExitCode::from(code as u8),
            Ok(Step::Asleep) => {
                // Nothing in Nebu can send an event yet, so the core sleeps forever,
                // as it would in QEMU.
                eprintln!("nebu: core 0 is waiting for an event that nothing can send");
                eprintln!("nebu: press Ctrl-C to quit");
                loop {
                    thread::park();
                }
            }
            Err(fault) => {
                eprintln!("nebu: stopped at pc {:#010x}: {fault}", cpu.pc);
                dump_registers(&cpu);
                return ExitCode::from(FAULT_EXIT_CODE);
            }
        }
    }
}

/// Starts passing standard input to the UART, as QEMU's `-serial stdio` does.
///
/// A separate thread does the reading, so the emulated CPU never waits for input.
fn read_stdin() -> Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut buffer = [0; 4096];
        while let Ok(count @ 1..) = stdin.read(&mut buffer) {
            if sender.send(buffer[..count].to_vec()).is_err() {
                break;
            }
        }
    });
    receiver
}

/// Prints the CPU state, to help work out what the software was doing.
fn dump_registers(cpu: &Cpu) {
    for (pair, values) in cpu.x.chunks(2).enumerate() {
        let line: Vec<String> = values
            .iter()
            .enumerate()
            .map(|(offset, value)| format!("x{:<2} {value:#018x}", pair * 2 + offset))
            .collect();
        eprintln!("  {}", line.join("   "));
    }
    let f = cpu.flags;
    let flag = |set: bool, name: char| if set { name } else { '-' };
    eprintln!(
        "  sp  {:#018x}   pc  {:#018x}   EL{}{}  {}{}{}{}",
        cpu.sp(),
        cpu.pc,
        cpu.el,
        if cpu.spsel { 'h' } else { 't' },
        flag(f.n, 'N'),
        flag(f.z, 'Z'),
        flag(f.c, 'C'),
        flag(f.v, 'V')
    );
}
