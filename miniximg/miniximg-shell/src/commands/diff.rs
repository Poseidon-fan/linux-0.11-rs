//! `diff` — line-based diff of two files (either side).
//!
//! Both operands may be either image or host paths. Files are read into
//! memory, decoded as UTF-8 (lossily, so binary data still produces some
//! output), then compared line by line via an LCS-driven algorithm.

use anyhow::Result;

use crate::{path, session::Session};

pub const NAME: &str = "diff";
pub const ALIASES: &[&str] = &[];
pub const SUMMARY: &str = "diff two files (image and/or host)";
pub const USAGE: &str = "diff A B";

pub fn run(session: &mut Session, args: &[String]) -> Result<()> {
    super::expect_args(args, 2, Some(2))?;

    let cwd = session.image_cwd().to_string();
    let host_cwd = session.host_cwd().to_path_buf();

    let a_bytes = read_any(session, &args[0], &cwd, &host_cwd)?;
    let b_bytes = read_any(session, &args[1], &cwd, &host_cwd)?;
    if a_bytes == b_bytes {
        return Ok(());
    }
    let a_text = String::from_utf8_lossy(&a_bytes).into_owned();
    let b_text = String::from_utf8_lossy(&b_bytes).into_owned();
    let a_lines: Vec<&str> = a_text.lines().collect();
    let b_lines: Vec<&str> = b_text.lines().collect();
    print_diff(&a_lines, &b_lines, &args[0], &args[1]);
    Ok(())
}

fn read_any(
    session: &mut Session,
    raw: &str,
    cwd: &str,
    host_cwd: &std::path::Path,
) -> Result<Vec<u8>> {
    match path::resolve(raw, cwd, host_cwd) {
        path::AnyPath::Image(p) => Ok(session.fs_mut().read_file_at_path(&p)?),
        path::AnyPath::Host(p) => Ok(std::fs::read(&p)?),
    }
}

/// Standard LCS-based line diff, printed in the classic `<` / `>` form
/// with a header line per hunk. Good enough for the small text files
/// people typically edit in this shell.
fn print_diff(a: &[&str], b: &[&str], a_name: &str, b_name: &str) {
    let n = a.len();
    let m = b.len();
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    println!("--- {}", a_name);
    println!("+++ {}", b_name);
    let mut i = 0;
    let mut j = 0;
    while i < n && j < m {
        if a[i] == b[j] {
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            println!("- {}", a[i]);
            i += 1;
        } else {
            println!("+ {}", b[j]);
            j += 1;
        }
    }
    while i < n {
        println!("- {}", a[i]);
        i += 1;
    }
    while j < m {
        println!("+ {}", b[j]);
        j += 1;
    }
}
