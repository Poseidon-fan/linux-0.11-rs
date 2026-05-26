//! `diff` — compare files line by line.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::String, vec::Vec};

use anyhow::Result;
use user_lib::{
    eprintln,
    fs::File,
    io::{self, BufRead, BufReader, Write},
    process::ExitCode,
};
use user_program::cli::cli_args;

cli_args! {
    /// Compare files line by line.
    pub struct DiffArgs {
        /// Files to compare.
        pub files: Vec<String> = [..] @ "FILE",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = DiffArgs::parse_env_or_exit();
    if cli.files.len() != 2 {
        eprintln!("diff: needs exactly two FILE arguments");
        return ExitCode::from(2);
    }
    let a = &cli.files[0];
    let b = &cli.files[1];
    let (lines_a, lines_b) = match (read_lines(a), read_lines(b)) {
        (Ok(la), Ok(lb)) => (la, lb),
        _ => return ExitCode::from(2),
    };

    let edits = lcs_diff(&lines_a, &lines_b);
    if edits.is_empty() {
        return ExitCode::SUCCESS;
    }
    print_normal_diff(&lines_a, &lines_b, &edits, a, b);
    ExitCode::from(1)
}

fn read_lines(path: &str) -> Result<Vec<String>> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut lines = Vec::new();
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        if line.ends_with('\n') {
            line.pop();
        }
        lines.push(line.clone());
    }
    Ok(lines)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Edit {
    Keep,
    Delete,
    Insert,
}

#[allow(clippy::needless_range_loop)]
fn lcs_diff(a: &[String], b: &[String]) -> Vec<Edit> {
    let n = a.len();
    let m = b.len();
    // DP table
    let mut dp = alloc::vec![alloc::vec![0usize; m + 1]; n + 1];
    for i in 0..n {
        for j in 0..m {
            if a[i] == b[j] {
                dp[i + 1][j + 1] = dp[i][j] + 1;
            } else {
                dp[i + 1][j + 1] = dp[i + 1][j].max(dp[i][j + 1]);
            }
        }
    }
    // Backtrack
    let mut edits = Vec::new();
    let (mut i, mut j) = (n, m);
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && a[i - 1] == b[j - 1] {
            edits.push(Edit::Keep);
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || dp[i][j - 1] >= dp[i - 1][j]) {
            edits.push(Edit::Insert);
            j -= 1;
        } else {
            edits.push(Edit::Delete);
            i -= 1;
        }
    }
    edits.reverse();
    edits
}

fn print_normal_diff(a: &[String], b: &[String], edits: &[Edit], _fa: &str, _fb: &str) {
    let mut out = io::stdout();
    let mut buf = String::new();
    use core::fmt::Write as _;

    let mut chunks: Vec<(usize, usize, usize, usize)> = Vec::new(); // a_start, a_end, b_start, b_end
    let mut i = 0usize;
    let mut ai = 0usize;
    let mut bi = 0usize;
    while i < edits.len() {
        while i < edits.len() && edits[i] == Edit::Keep {
            ai += 1;
            bi += 1;
            i += 1;
        }
        let ctx_start = i;
        while i < edits.len() && edits[i] != Edit::Keep {
            i += 1;
        }
        if ctx_start < i {
            let ca = (ctx_start..i).filter(|&k| edits[k] == Edit::Delete).count();
            let cb = (ctx_start..i).filter(|&k| edits[k] == Edit::Insert).count();
            chunks.push((ai, ai + ca, bi, bi + cb));
            ai += ca;
            bi += cb;
        }
    }

    if chunks.is_empty() {
        return;
    }
    let _ = writeln!(buf, "{} c {}", chunks.len(), chunks.len());

    for (as_, ae, bs, be) in &chunks {
        let _ = writeln!(buf, "{},{}c{},{}", as_ + 1, ae, bs + 1, be);
        for line in &a[*as_..*ae] {
            let _ = writeln!(buf, "< {}", line);
        }
        let _ = writeln!(buf, "---");
        for line in &b[*bs..*be] {
            let _ = writeln!(buf, "> {}", line);
        }
    }
    let _ = out.write_all(buf.as_bytes());
}
