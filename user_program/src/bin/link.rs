//! `link` — create a hard link (low-level interface, no flags).

#![no_std]
#![no_main]

extern crate alloc;

use anyhow::Result;
use user_lib::{eprintln, fs};
use user_program::cli::cli_args;

cli_args! {
    /// Create a hard link named FILE2 to FILE1.
    pub struct LinkArgs {
        /// Existing file the new link will point to.
        pub file1: Option<alloc::string::String> = [..],
    }
}

#[user_lib::main]
fn main() -> Result<()> {
    // Bypass the macro's positional sink for two-arg fixed form.
    let mut args = user_lib::env::args().skip(1);
    let file1 = args.next();
    let file2 = args.next();
    let extra = args.next();

    let (file1, file2) = match (file1.as_deref(), file2.as_deref(), extra) {
        (None, _, _) | (_, None, _) => {
            eprintln!("link: missing operand");
            anyhow::bail!("usage: link FILE1 FILE2");
        }
        (_, _, Some(_)) => {
            eprintln!("link: extra operand");
            anyhow::bail!("usage: link FILE1 FILE2");
        }
        (Some(a), Some(b), None) => (a, b),
    };

    fs::hard_link(file1, file2)
        .map_err(|err| anyhow::anyhow!("cannot create link '{}' to '{}': {}", file2, file1, err))?;

    Ok(())
}
