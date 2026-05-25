//! `sync` — flush filesystem buffers to disk.

#![no_std]
#![no_main]

extern crate alloc;

use anyhow::Result;
use user_lib::syscall;

#[user_lib::main]
fn main() -> Result<()> {
    syscall::fs::sync().map_err(|err| anyhow::anyhow!("sync failed: errno {}", err.code()))?;
    Ok(())
}
