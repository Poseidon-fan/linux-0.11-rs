//! CLI definitions for `mbrkit`.
//!
//! The command tree is intentionally thin and hands raw values to the library
//! modules for validation and execution.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

/// The top-level command line parser.
#[derive(Debug, Parser)]
#[command(
    name = "mbrkit",
    version,
    about = "Build and inspect MBR-backed disk images"
)]
pub struct Cli {
    /// The subcommand to execute.
    #[command(subcommand)]
    pub command: Command,
}

/// The supported top-level commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Pack one or more partition images into an MBR disk image.
    Pack(PackArgs),
    /// Inspect an existing disk image and print its MBR layout.
    Inspect(InspectArgs),
    /// Extract one partition payload from an existing disk image.
    Extract(ExtractArgs),
    /// Validate an existing disk image and return an appropriate exit code.
    Verify(VerifyArgs),
}

/// Output formats shared by read-only commands.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum OutputFormat {
    /// Print a human-friendly table report.
    Table,
    /// Print a machine-friendly JSON report.
    Json,
}

/// Arguments for the `pack` command.
#[derive(Debug, Args)]
pub struct PackArgs {
    /// Write the disk image described by a TOML manifest.
    #[arg(long, value_name = "FILE", conflicts_with_all = [
        "output",
        "disk_size",
        "boot_code",
        "disk_signature",
        "align",
        "partition"
    ])]
    pub manifest: Option<PathBuf>,

    /// The output disk image path when using explicit CLI flags.
    #[arg(short, long, value_name = "FILE", required_unless_present = "manifest")]
    pub output: Option<PathBuf>,

    /// The final logical disk size, such as `32MiB` or `64M`.
    #[arg(long, value_name = "SIZE", required_unless_present = "manifest")]
    pub disk_size: Option<String>,

    /// An optional MBR bootstrap code image. The payload must fit in 440 bytes.
    #[arg(long, value_name = "FILE")]
    pub boot_code: Option<PathBuf>,

    /// An optional disk signature written to bytes 440..444 in the MBR sector.
    #[arg(long, value_name = "HEX_OR_DECIMAL")]
    pub disk_signature: Option<String>,

    /// Alignment in sectors used by auto-placed partitions.
    #[arg(long, value_name = "SECTORS", default_value_t = 2048)]
    pub align: u64,

    /// A repeated partition description in `key=value` form.
    #[arg(long, value_name = "SPEC", required_unless_present = "manifest")]
    pub partition: Vec<String>,

    /// Print the resolved layout without creating the disk image.
    #[arg(long)]
    pub dry_run: bool,

    /// Overwrite an existing output file.
    #[arg(long)]
    pub force: bool,
}

/// Arguments for the `inspect` command.
#[derive(Debug, Args)]
pub struct InspectArgs {
    /// The disk image to inspect.
    #[arg(value_name = "DISK")]
    pub disk: PathBuf,

    /// The desired report format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

/// Arguments for the `extract` command.
#[derive(Debug, Args)]
pub struct ExtractArgs {
    /// The disk image to inspect.
    #[arg(value_name = "DISK")]
    pub disk: PathBuf,

    /// The one-based partition number to extract.
    #[arg(long, value_name = "INDEX")]
    pub partition: usize,

    /// The output image path for the extracted payload.
    #[arg(short, long, value_name = "FILE")]
    pub output: PathBuf,

    /// Overwrite an existing output file.
    #[arg(long)]
    pub force: bool,
}

/// Arguments for the `verify` command.
#[derive(Debug, Args)]
pub struct VerifyArgs {
    /// The disk image to validate.
    #[arg(value_name = "DISK")]
    pub disk: PathBuf,

    /// The desired report format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,

    /// Treat warnings as failures.
    #[arg(long)]
    pub strict: bool,
}
