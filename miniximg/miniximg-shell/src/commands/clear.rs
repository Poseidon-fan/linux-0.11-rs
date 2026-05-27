//! `clear` — clear the terminal screen.
//!
//! Emits the standard ANSI sequence `ESC [ 2 J ESC [ H` (erase entire
//! display, then home the cursor) and flushes stdout. This works on
//! every VT100-compatible terminal — which is what users of an
//! interactive shell are running by definition.

use std::io::Write;

use anyhow::Result;

use crate::session::Session;

pub const NAME: &str = "clear";
pub const ALIASES: &[&str] = &[];
pub const SUMMARY: &str = "clear the terminal screen";
pub const USAGE: &str = "clear";

pub fn run(_session: &mut Session, _args: &[String]) -> Result<()> {
    let mut out = std::io::stdout().lock();
    out.write_all(b"\x1b[2J\x1b[H")?;
    out.flush()?;
    Ok(())
}
