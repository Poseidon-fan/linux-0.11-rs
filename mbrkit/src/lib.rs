//! The library crate hosts all reusable logic for the `mbrkit` binary.
//!
//! The binary stays intentionally small so that command parsing, image layout
//! building, MBR encoding, and report generation can be tested directly.

pub mod cli;
mod commands;
pub mod error;
pub mod layout;
pub mod manifest;
pub mod mbr;
pub mod report;

pub use error::{MbrkitError, Result};

use clap::Parser;

use crate::cli::Cli;

/// Parse CLI arguments and execute the selected command.
pub fn run() -> Result<()> {
    let cli = Cli::parse();
    commands::run(cli)
}
