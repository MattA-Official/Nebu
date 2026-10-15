//! Nebuchadnezzar (Nebu): a minimal Raspberry Pi 3B emulator.
//!
//! Nebu emulates only the hardware that NeOS uses, and halts loudly on anything else.
//! See `docs/decisions/` for the reasoning behind its design.

#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    match args.next() {
        Some(kernel) => {
            eprintln!("nebu: cannot run {kernel} yet; the CPU core arrives with NeOS step 2");
            ExitCode::FAILURE
        }
        None => {
            eprintln!("usage: nebu <kernel8.img>");
            ExitCode::from(2)
        }
    }
}
