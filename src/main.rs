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
mod rng;
mod system_timer;
mod usb;
mod usb_devices;
mod usb_network;

use std::io::Read;
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver};
use std::{env, fs, thread};

use bus::Bus;
use cpu::{Cpu, Step};
use emmc::{Card, Emmc};
use pl011::Pl011;
use usb::Dwc2;

/// Where the firmware loads `kernel8.img` and starts running it.
const LOAD_ADDRESS: u64 = 0x80000;

/// Exit code when Nebu meets something it cannot emulate.
const FAULT_EXIT_CODE: u8 = 70;

const USAGE: &str = "\
usage: nebu [--semihosting] [--trace] [--sd <card.img>] [--load <file>@<address>]...
            [--screen <file.ppm>] [--keyboard] [--keys <list>] [--network]
            <kernel8.img>

  --semihosting  let `hlt #0xf000` make semihosting calls, as QEMU's -semihosting does
  --trace        print every instruction to standard error before running it
  --sd <file>    put an SD card holding <file> in the slot; it must be a whole
                 number of 512 KiB, from 2 GiB to 32 GiB, and blocks written to
                 the card are written to <file>
  --load <file>@<address>
                 copy <file> into memory at <address> (hexadecimal, with 0x)
                 before starting, as the firmware does with config.txt's
                 initramfs, and QEMU with -device loader
  --screen <file.ppm>
                 when the machine stops, save what the screen shows to <file.ppm>,
                 as QEMU's screendump does
  --keyboard     plug a USB keyboard into the USB port, behind a hub, as QEMU's
                 -device usb-kbd does
  --keys <list>  plug in the keyboard, and type on it: each time NeOS asks it for a
                 report, the next of these presses (+name) and releases (-name),
                 separated by commas, with QEMU's names for keys, such as
                 +shift,+h,-h,-shift; ~N holds the keys as they are for the next N
                 times it asks
  --network      plug a USB network card into the USB port, behind a hub, on a
                 network whose gateway answers ARP, as QEMU's -device usb-net with
                 -netdev user does; not with a keyboard";

fn main() -> ExitCode {
    let mut semihosting = false;
    let mut trace = false;
    let mut card = None;
    let mut loads = Vec::new();
    let mut screen = None;
    let mut keyboard = None;
    let mut network = false;
    let mut image = None;
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--semihosting" => semihosting = true,
            "--trace" => trace = true,
            "--keyboard" => {
                keyboard.get_or_insert_with(Vec::new);
            }
            "--network" => network = true,
            "--keys" => match arguments.next().as_deref().map(usb_devices::parse_keys) {
                Some(Ok(keys)) => keyboard.get_or_insert_with(Vec::new).extend(keys),
                Some(Err(error)) => {
                    eprintln!("nebu: --keys: {error}");
                    return ExitCode::from(2);
                }
                None => {
                    eprintln!("{USAGE}");
                    return ExitCode::from(2);
                }
            },
            "--sd" if card.is_none() => match arguments.next() {
                Some(path) => card = Some(path),
                None => {
                    eprintln!("{USAGE}");
                    return ExitCode::from(2);
                }
            },
            "--screen" if screen.is_none() => match arguments.next() {
                Some(path) => screen = Some(path),
                None => {
                    eprintln!("{USAGE}");
                    return ExitCode::from(2);
                }
            },
            "--load" => match arguments.next().as_deref().and_then(parse_load) {
                Some(load) => loads.push(load),
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

    if network && keyboard.is_some() {
        eprintln!("nebu: --network cannot go with a keyboard: Nebu's hub has one device");
        return ExitCode::from(2);
    }
    let card = match card.map(|path| open_card(&path)).transpose() {
        Ok(card) => card,
        Err(error) => {
            eprintln!("nebu: {error}");
            return ExitCode::from(2);
        }
    };
    let uart = Pl011::new(Box::new(std::io::stdout()), read_stdin());
    let mut bus = Bus::with_devices(uart, Emmc::new(card), Dwc2::new(keyboard, network));
    if let Err(error) = bus.load(LOAD_ADDRESS, &bytes) {
        eprintln!("nebu: cannot load {image}: {error}");
        return ExitCode::from(2);
    }
    for (path, address) in loads {
        let loaded = fs::read(&path)
            .map_err(|error| error.to_string())
            .and_then(|bytes| bus.load(address, &bytes).map_err(|error| error.to_string()));
        if let Err(error) = loaded {
            eprintln!("nebu: cannot load {path} at {address:#x}: {error}");
            return ExitCode::from(2);
        }
    }
    // Core 0 starts at the kernel, as the firmware leaves it; the others wait in the
    // firmware's stub for the spin table (ADR 0030).
    let mut cores = vec![Core { cpu: Cpu::new(LOAD_ADDRESS, semihosting), state: State::Running }];
    for core in 1..CORES {
        cores.push(Core { cpu: Cpu::in_stub(core, semihosting), state: State::InStub });
    }
    let mut now = 0;

    loop {
        // One count of the counter is one instruction on every core that runs (ADRs
        // 0016 and 0030). The cores take turns in order, one instruction each, so a
        // run gives the same result every time.
        now += 1;
        bus.advance(now);
        let mut events = false;
        for core in &mut cores {
            match core.state {
                State::Running => {}
                State::WaitingForInterrupt if core.cpu.irq_pending(&mut bus, now) => {
                    core.state = State::Running;
                }
                State::WaitingForEvent if core.cpu.take_event() => core.state = State::Running,
                State::InStub if core.cpu.take_event() => {
                    let word = SPIN_TABLE + 8 * u64::from(core.cpu.core);
                    let entry = bus.read(word, 8).unwrap_or(0);
                    if entry != 0 {
                        core.cpu.leave_stub(entry, SPIN_TABLE);
                        core.state = State::Running;
                    }
                }
                State::WaitingForInterrupt | State::WaitingForEvent | State::InStub => continue,
            }
            if core.state != State::Running {
                continue;
            }
            match core.cpu.step(&mut bus, now, trace) {
                Ok(Step::Continue) => {}
                Ok(Step::Event) => events = true,
                Ok(Step::Asleep) => core.state = State::WaitingForEvent,
                Ok(Step::WaitingForInterrupt) => core.state = State::WaitingForInterrupt,
                Ok(Step::Exit(code)) => {
                    save_screen(&bus, screen.as_deref());
                    return ExitCode::from(code as u8);
                }
                Err(fault) => {
                    eprintln!(
                        "nebu: core {} stopped at pc {:#010x}: {fault}",
                        core.cpu.core, core.cpu.pc
                    );
                    dump_registers(&core.cpu);
                    save_screen(&bus, screen.as_deref());
                    return ExitCode::from(FAULT_EXIT_CODE);
                }
            }
        }
        if events {
            cores.iter_mut().for_each(|core| core.cpu.send_event());
        }
        if cores.iter().any(|core| core.state == State::Running) {
            continue;
        }

        // Every core waits. If one waits for an interrupt that is due, skip straight
        // to it: the next count finds it raised (ADR 0017).
        let due = cores
            .iter()
            .filter(|core| core.state == State::WaitingForInterrupt)
            .filter_map(|core| core.cpu.next_interrupt(&bus))
            .min();
        match due {
            // A deadline that has passed is already due.
            Some(due) => now = now.max(due.saturating_sub(1)),
            // With nothing due, only input to the UART can wake a core (ADR 0018).
            None if cores.iter().any(|core| {
                core.state == State::WaitingForInterrupt && core.cpu.takes_device_interrupts()
            }) && bus.wait_for_input() => {}
            None => {
                // Nothing will change the screen now, so it is saved before waiting.
                save_screen(&bus, screen.as_deref());
                // No core can send an event, and no interrupt is due or can be raised
                // by input, so the cores sleep forever, as they would in QEMU.
                eprintln!("nebu: every core is waiting for something that nothing can send");
                eprintln!("nebu: press Ctrl-C to quit");
                loop {
                    thread::park();
                }
            }
        }
    }
}

/// How many cores the BCM2837 has (ADR 0030).
const CORES: u8 = 4;

/// The firmware's spin table: core n waits in the stub until the word at this address
/// plus 8n holds where to start (ADR 0030).
const SPIN_TABLE: u64 = 0xD8;

/// One core, and what it is doing.
struct Core {
    cpu: Cpu,
    state: State,
}

/// What a core is doing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    /// Running instructions.
    Running,
    /// In `wfi`, until an interrupt is pending.
    WaitingForInterrupt,
    /// In `wfe`, until an event.
    WaitingForEvent,
    /// In the firmware's stub, until an event finds an address in the spin table.
    InStub,
}

/// Saves what the screen shows to `path`, if one was asked for, as a binary PPM
/// image (ADR 0023). With no framebuffer, it says so and saves nothing.
fn save_screen(bus: &Bus, path: Option<&str>) {
    let Some(path) = path else {
        return;
    };
    let Some((width, height, pixels)) = bus.screen() else {
        eprintln!("nebu: no screen to save: the firmware was never asked for a framebuffer");
        return;
    };
    let mut image = format!("P6\n{width} {height}\n255\n").into_bytes();
    image.extend_from_slice(&pixels);
    if let Err(error) = fs::write(path, image) {
        eprintln!("nebu: cannot save the screen to {path}: {error}");
    }
}

/// Splits a `--load` argument, `<file>@<address>`, into the file and the address.
fn parse_load(argument: &str) -> Option<(String, u64)> {
    let (path, address) = argument.rsplit_once('@')?;
    let address = u64::from_str_radix(address.strip_prefix("0x")?, 16).ok()?;
    (!path.is_empty()).then(|| (path.to_string(), address))
}

/// Opens the SD card image at `path`, for reading and writing (ADR 0028).
fn open_card(path: &str) -> Result<Card, String> {
    // An image that may only be read still serves a kernel that only reads; a write
    // to it halts (ADR 0028).
    let file = fs::File::options()
        .read(true)
        .write(true)
        .open(path)
        .or_else(|_| fs::File::open(path))
        .map_err(|error| format!("cannot open {path}: {error}"))?;
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

#[cfg(test)]
mod tests {
    use super::parse_load;

    #[test]
    fn load_arguments_split_at_the_last_at_sign() {
        assert_eq!(parse_load("neos.img@0x3000000"), Some(("neos.img".into(), 0x300_0000)));
        assert_eq!(parse_load("a@b.img@0x10"), Some(("a@b.img".into(), 0x10)));
    }

    #[test]
    fn load_arguments_need_a_file_and_a_hexadecimal_address() {
        assert_eq!(parse_load("neos.img"), None);
        assert_eq!(parse_load("neos.img@3000000"), None);
        assert_eq!(parse_load("@0x3000000"), None);
        assert_eq!(parse_load("neos.img@0xzz"), None);
    }
}
