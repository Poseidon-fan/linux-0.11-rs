//! `clear` — clear the terminal screen using ANSI escape sequences.

#![no_std]
#![no_main]

extern crate alloc;

use anyhow::Result;
use user_lib::io::{self, Write};

/// Cursor home + erase entire display + erase scrollback (xterm-style).
const CLEAR_SEQ: &[u8] = b"\x1b[H\x1b[2J\x1b[3J";

#[user_lib::main]
fn main() -> Result<()> {
    io::stdout().write_all(CLEAR_SEQ)?;
    Ok(())
}
