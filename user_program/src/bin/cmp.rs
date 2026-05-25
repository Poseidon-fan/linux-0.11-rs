//! `cmp` — compare two files byte by byte.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::String, vec::Vec};

use anyhow::Result;
use user_lib::{
    eprintln,
    fs::File,
    io::{self, Read, Write},
    process::ExitCode,
};
use user_program::cli::cli_args;

cli_args! {
    /// Compare two files byte by byte.
    pub struct CmpArgs {
        /// Print nothing for differing files; only set the exit status.
        pub silent:  bool       = ["-s", "--silent", "--quiet"],
        /// Output byte numbers and differing byte values of every difference.
        pub list:    bool       = ["-l", "--verbose"],
        /// Compare at most NUM bytes.
        pub limit:   u32        = ["-n", "--bytes"] @ "NUM" = u32::MAX,
        /// The two files (and optional skip offsets, ignored for now).
        pub files:   Vec<String> = [..] @ "FILE",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = CmpArgs::parse_env_or_exit();
    if cli.files.len() != 2 {
        eprintln!("cmp: needs exactly two FILE arguments");
        return ExitCode::from(2);
    }
    let (a, b) = (cli.files[0].as_str(), cli.files[1].as_str());

    let fa = match File::open(a) {
        Ok(f) => f,
        Err(err) => {
            eprintln!("cmp: {}: {}", a, err);
            return ExitCode::from(2);
        }
    };
    let fb = match File::open(b) {
        Ok(f) => f,
        Err(err) => {
            eprintln!("cmp: {}: {}", b, err);
            return ExitCode::from(2);
        }
    };

    match compare(fa, fb, cli.limit, cli.silent, cli.list) {
        Ok(Outcome::Same) => ExitCode::SUCCESS,
        Ok(Outcome::Differ) => ExitCode::from(1),
        Ok(Outcome::ShortFile { which, byte, line }) => {
            if !cli.silent {
                let name = if which == 0 { a } else { b };
                eprintln!(
                    "cmp: EOF on {} after byte {}, line {}",
                    name,
                    byte - 1,
                    line
                );
            }
            ExitCode::from(1)
        }
        Err(err) => {
            eprintln!("cmp: {}", err);
            ExitCode::from(2)
        }
    }
}

enum Outcome {
    Same,
    Differ,
    ShortFile { which: u8, byte: u32, line: u32 },
}

fn compare<A: Read, B: Read>(
    mut a: A,
    mut b: B,
    limit: u32,
    silent: bool,
    list: bool,
) -> Result<Outcome> {
    let mut buf_a = [0u8; 1024];
    let mut buf_b = [0u8; 1024];
    let mut byte: u32 = 1; // POSIX byte counts are 1-based
    let mut line: u32 = 1;
    let mut remaining = limit;
    let mut printed_any = false;
    let mut out = io::stdout();

    while remaining > 0 {
        let want = core::cmp::min(remaining, buf_a.len() as u32) as usize;
        let na = read_at_most(&mut a, &mut buf_a[..want])?;
        let nb = read_at_most(&mut b, &mut buf_b[..want])?;
        let common = core::cmp::min(na, nb);

        for i in 0..common {
            let byte_a = buf_a[i];
            let byte_b = buf_b[i];
            if byte_a != byte_b {
                if list {
                    let mut line_buf = String::new();
                    use core::fmt::Write as _;
                    let _ = writeln!(line_buf, "{:>7} {:>3o} {:>3o}", byte, byte_a, byte_b);
                    out.write_all(line_buf.as_bytes())?;
                    printed_any = true;
                } else {
                    if !silent {
                        // GNU: `a b differ: byte 5, line 1`
                        let mut line_buf = String::new();
                        use core::fmt::Write as _;
                        let _ = writeln!(line_buf, "differ: byte {}, line {}", byte, line);
                        out.write_all(line_buf.as_bytes())?;
                    }
                    return Ok(Outcome::Differ);
                }
            }
            if byte_a == b'\n' {
                line += 1;
            }
            byte += 1;
        }

        if na != nb {
            // One file ran out first.
            let (shorter, bytes_consumed) = if na < nb { (0u8, na) } else { (1u8, nb) };
            // adjust line counter for the part that DID match in the longer file
            for &c in &buf_a[..bytes_consumed] {
                if c == b'\n' { /* already counted above */ }
                let _ = c;
            }
            return Ok(Outcome::ShortFile {
                which: shorter,
                byte,
                line,
            });
        }

        if na == 0 {
            break;
        }
        remaining -= na as u32;
    }

    if printed_any {
        Ok(Outcome::Differ)
    } else {
        Ok(Outcome::Same)
    }
}

fn read_at_most<R: Read>(reader: &mut R, buf: &mut [u8]) -> Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        match reader.read(&mut buf[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err.into()),
        }
    }
    Ok(total)
}
