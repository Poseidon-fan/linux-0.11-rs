//! Interactive shell for [Minix v1 filesystem images][minix].
//!
//! `miniximg shell foo.img` drops into a REPL where paths default to the
//! image. Anything prefixed with `@` is interpreted as a host path
//! instead. Two working directories are tracked independently — `cd`
//! changes the image-side cwd, `lcd` changes the host-side one — so the
//! user can move around both filesystems in parallel.
//!
//! ```no_run
//! use miniximg_shell::{run, ShellOptions};
//! use std::path::PathBuf;
//!
//! run(ShellOptions {
//!     image: PathBuf::from("disk.img"),
//!     readonly: false,
//!     history: None,
//! })?;
//! # Ok::<(), anyhow::Error>(())
//! ```
//!
//! Modifications are auto-committed to the image file as each command
//! returns. Pass `readonly: true` to refuse every mutating command.
//!
//! [minix]: https://en.wikipedia.org/wiki/MINIX_file_system

mod commands;
mod editor;
mod parser;
mod path;
mod session;

pub use session::ShellOptions;

use anyhow::Result;

/// Run the interactive shell against the configured image, returning
/// when the user issues `exit`, `quit`, or `Ctrl-D` on an empty line.
pub fn run(options: ShellOptions) -> Result<()> {
    let session = session::Session::open(options)?;
    editor::run_repl(session)?;
    Ok(())
}
