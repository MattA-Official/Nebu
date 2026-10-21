//! Nebuchadnezzar (Nebu): a minimal Raspberry Pi 3B emulator.
//!
//! Nebu emulates only the hardware that NeOS uses, and halts loudly on anything else.
//! See `docs/decisions/` for the reasoning behind its design.

#![forbid(unsafe_code)]

mod bus;
mod cpu;
mod decode;
mod emmc;
mod gpio;
mod local;
mod mailbox;
mod mmu;
mod pl011;
mod system_timer;

use std::io::Read;
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver};
use std::{env, fs, thread};

use bus::Bus;
use cpu::{Cpu, Step};
use emmc::{Card, Emmc};
use pl011::Pl011;

/// Where the firmware loads `kernel8.img` and starts running it.
const LOAD_ADDRESS: u64 = 0x80000;

/// Exit code when Nebu meets something it cannot emulate.
const FAULT_EXIT_CODE: u8 = 70;

const USAGE: &str = "\
usage: nebu [--semihosting] [--trace] [--sd <card.img>] <kernel8.img>

  --semihosting  let `hlt #0xf000` make semihosting calls, as QEMU's -semihosting does
  --trace        print every instruction to standard error before running it
  --sd <file>    put an SD card holding <file> in the slot; it must be a whole
                 number of 512 KiB, from 2 GiB to 32 GiB, and is only ever read";

fn main() -> ExitCode {
    let mut semihosting = false;
    let mut trace = false;
    let mut card = None;
    let mut image = None;
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--semihosting" => semihosting = true,
            "--trace" => trace = true,
            "--sd" if card.is_none() => match arguments.next() {
                Some(path) => card = Some(path),
                None => {
                    eprintln!("{USAGE}");
                    return ExitCode::from(2);
                }
            },
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

    let card = match card.map(|path| open_card(&path)).transpose() {
        Ok(card) => card,
        Err(error) => {
            eprintln!("nebu: {error}");
            return ExitCode::from(2);
        }
    };
    let uart = Pl011::new(Box::new(std::io::stdout()), read_stdin());
    let mut bus = Bus::with_emmc(uart, Emmc::new(card));
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
                // Nothing in Nebu can send an event, and no interrupt is due or can be
                // raised by input, so the core sleeps forever, as it would in QEMU.
                eprintln!("nebu: core 0 is waiting for something that nothing can send");
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

/// Opens the SD card image at `path`, for reading only.
fn open_card(path: &str) -> Result<Card, String> {
    let file = fs::File::open(path).map_err(|error| format!("cannot read {path}: {error}"))?;
    Card::new(file).map_err(|error| format!("{path}: {error}"))
}

/// Starts passing standard input to the UART, as QEMU's `-serial stdio` does.
///
/// A separate thread does the reading, so the emulated CPU waits for input only
/// when it has nothing else to wait for (ADR 0018).
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
