//! `yes` — repeatedly output a line until killed.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::String, vec::Vec};

use anyhow::Result;
use user_lib::io::{self, Write};
use user_program::cli::cli_args;

cli_args! {
    /// Repeatedly output a line with all specified STRING(s), or `y`.
    pub struct YesArgs {
        /// Strings to repeat; defaults to `y` when empty.
        pub strings: Vec<String> = [..] @ "STRING",
    }
}

#[user_lib::main]
fn main() -> Result<()> {
    let cli = YesArgs::parse_env_or_exit();

    let mut line = if cli.strings.is_empty() {
        String::from("y")
    } else {
        cli.strings.join(" ")
    };
    line.push('\n');

    let mut stdout = io::stdout();
    loop {
        stdout.write_all(line.as_bytes())?;
    }
}
