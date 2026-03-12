//! Command dispatch and shared helpers.

mod extract;
mod inspect;
mod pack;
mod verify;

use crate::cli::{Cli, Command};
use crate::error::Result;

/// Execute the CLI-selected command.
pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Pack(args) => pack::run(args),
        Command::Inspect(args) => inspect::run(args),
        Command::Extract(args) => extract::run(args),
        Command::Verify(args) => verify::run(args),
    }
}
