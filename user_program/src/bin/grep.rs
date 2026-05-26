//! `grep` — search for patterns in files (fixed-string mode).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use anyhow::{Context, Result};
use user_lib::{
    eprintln,
    fs::{self, File},
    io::{self, BufRead, BufReader, Write},
    process::ExitCode,
};
use user_program::cli::cli_args;

cli_args! {
    pub struct GrepArgs {
        pub ignore_case:  bool           = ["-i", "--ignore-case"],
        pub invert:       bool           = ["-v", "--invert-match"],
        pub count:        bool           = ["-c", "--count"],
        pub line_number:  bool           = ["-n", "--line-number"],
        pub recursive:    bool           = ["-r", "-R", "--recursive"],
        pub files_with:   bool           = ["-l", "--files-with-matches"],
        pub no_filename:  bool           = ["-h", "--no-filename"],
        pub with_filename:bool           = ["-H", "--with-filename"],
        pub quiet:        bool           = ["-q", "--quiet", "--silent"],
        pub word_regexp:  bool           = ["-w", "--word-regexp"],
        pub line_regexp:  bool           = ["-x", "--line-regexp"],
        pub extended:     bool           = ["-E", "--extended-regexp"],
        pub after:        Option<String> = ["-A", "--after-context"] @ "NUM",
        pub before:       Option<String> = ["-B", "--before-context"] @ "NUM",
        pub context:      Option<String> = ["-C", "--context"] @ "NUM",
        pub pattern:      Option<String> = ["-e", "--regexp"] @ "PATTERN",
        pub args:         Vec<String>    = [..] @ "ARG",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = GrepArgs::parse_env_or_exit();
    let mut args = cli.args.iter();
    let pattern: String = if let Some(ref p) = cli.pattern {
        p.clone()
    } else if let Some(p) = args.next() {
        p.clone()
    } else {
        eprintln!("grep: missing pattern");
        return ExitCode::from(2);
    };
    let files: Vec<&str> = args.map(String::as_str).collect();
    let (after, before) = compute_context(&cli);
    let show_name = cli.with_filename || (!cli.no_filename && (files.len() > 1 || cli.recursive));
    let mut matched_any = false;
    let matcher = compile_regex(&pattern, &cli);
    let mut ctx = GrepCtx {
        cli: &cli,
        pattern: &pattern,
        matcher,
        show_name,
        after,
        before,
        matched_any: &mut matched_any,
    };

    if files.is_empty() {
        let _ = grep_reader(io::stdin(), "(standard input)", &mut ctx);
    } else {
        for path in &files {
            let _ = grep_path(path, &mut ctx);
            if cli.quiet && *ctx.matched_any {
                return ExitCode::SUCCESS;
            }
        }
    }
    if *ctx.matched_any || cli.count {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

struct GrepCtx<'a> {
    cli: &'a GrepArgs,
    pattern: &'a str,
    matcher: Matcher,
    show_name: bool,
    after: usize,
    before: usize,
    matched_any: &'a mut bool,
}

fn compute_context(cli: &GrepArgs) -> (usize, usize) {
    let c: usize = parse_num(cli.context.as_deref()).unwrap_or(0);
    (
        parse_num(cli.after.as_deref()).unwrap_or(0).max(c),
        parse_num(cli.before.as_deref()).unwrap_or(0).max(c),
    )
}

fn grep_path(path: &str, ctx: &mut GrepCtx<'_>) -> Result<()> {
    let meta = fs::metadata(path).with_context(|| path.to_string())?;
    if meta.is_dir() {
        if ctx.cli.recursive {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                let child = entry.path();
                let name = entry.file_name();
                if name == "." || name == ".." {
                    continue;
                }
                grep_path(child.as_str(), ctx).ok();
                if ctx.cli.quiet && *ctx.matched_any {
                    return Ok(());
                }
            }
        }
        return Ok(());
    }
    let file = File::open(path).with_context(|| path.to_string())?;
    grep_reader(file, path, ctx)
}

fn grep_reader<R: io::Read>(reader: R, name: &str, ctx: &mut GrepCtx<'_>) -> Result<()> {
    let mut buf = BufReader::new(reader);
    let mut line = String::new();
    let mut n = 0usize;
    let mut mc = 0usize;
    let mut ar = 0usize;
    let mut bb: Vec<String> = Vec::new();
    let mut out = io::stdout();
    loop {
        line.clear();
        if buf.read_line(&mut line)? == 0 {
            break;
        }
        n += 1;
        if line.ends_with('\n') {
            line.pop();
        }
        let m = line_matches(&line, ctx.pattern, ctx.cli, &ctx.matcher);
        if m {
            *ctx.matched_any = true;
            mc += 1;
        }
        if ar > 0 {
            print_line(&mut out, &line, name, n, ctx.cli, ctx.show_name);
            ar -= 1;
            if m {
                ar = ctx.after;
            }
            continue;
        }
        if m {
            if ctx.before > 0 && mc > 1 {
                let _ = out.write_all(b"--\n");
            }
            for bl in bb.drain(..) {
                print_line(&mut out, &bl, name, 0, ctx.cli, ctx.show_name);
            }
            print_line(&mut out, &line, name, n, ctx.cli, ctx.show_name);
            ar = ctx.after;
        } else if ctx.before > 0 {
            if bb.len() >= ctx.before {
                bb.remove(0);
            }
            bb.push(line.clone());
        }
        if ctx.cli.quiet && *ctx.matched_any {
            return Ok(());
        }
    }
    if ctx.cli.count {
        let label = if ctx.cli.files_with {
            alloc::format!("{}\n", name)
        } else if ctx.show_name {
            alloc::format!("{}:{}\n", name, mc)
        } else {
            alloc::format!("{}\n", mc)
        };
        let _ = out.write_all(label.as_bytes());
        if mc > 0 {
            *ctx.matched_any = true;
        }
    } else if ctx.cli.files_with && mc > 0 {
        let _ = out.write_all(name.as_bytes());
        let _ = out.write_all(b"\n");
    }
    Ok(())
}

enum Matcher {
    Fixed,
    Regex(regex::Regex),
}

fn compile_regex(pat: &str, cli: &GrepArgs) -> Matcher {
    if !cli.extended {
        return Matcher::Fixed;
    }
    let mut p = if cli.word_regexp {
        alloc::format!("\\b{}\\b", pat)
    } else if cli.line_regexp {
        alloc::format!("^{}$", pat)
    } else {
        String::from(pat)
    };
    if cli.ignore_case {
        p = alloc::format!("(?i){}", p);
    }
    match regex::Regex::new(&p) {
        Ok(re) => Matcher::Regex(re),
        Err(_) => Matcher::Fixed,
    }
}

fn line_matches(line: &str, pat: &str, cli: &GrepArgs, matcher: &Matcher) -> bool {
    if cli.extended {
        let m = match matcher {
            Matcher::Regex(re) => re.is_match(line),
            Matcher::Fixed => {
                if cli.line_regexp {
                    line == pat
                } else if cli.word_regexp {
                    word_match(line, pat, cli.ignore_case)
                } else if cli.ignore_case {
                    line.to_ascii_lowercase()
                        .contains(&pat.to_ascii_lowercase())
                } else {
                    line.contains(pat)
                }
            }
        };
        if cli.invert { !m } else { m }
    } else {
        let m = if cli.line_regexp {
            line == pat || (cli.ignore_case && line.eq_ignore_ascii_case(pat))
        } else if cli.word_regexp {
            word_match(line, pat, cli.ignore_case)
        } else if cli.ignore_case {
            line.to_ascii_lowercase()
                .contains(&pat.to_ascii_lowercase())
        } else {
            line.contains(pat)
        };
        if cli.invert { !m } else { m }
    }
}

fn word_match(haystack: &str, needle: &str, ic: bool) -> bool {
    if needle.is_empty() {
        return false;
    }
    let hb = haystack.as_bytes();
    let nb = needle.as_bytes();
    let mut pos = 0usize;
    while pos + nb.len() <= hb.len() {
        if &hb[pos..pos + nb.len()] == nb
            || (ic && hb[pos..pos + nb.len()].eq_ignore_ascii_case(nb))
        {
            let before = pos == 0 || is_word_bound(hb[pos - 1]);
            let after = pos + nb.len() == hb.len() || is_word_bound(hb[pos + nb.len()]);
            if before && after {
                return true;
            }
        }
        pos += 1;
    }
    false
}
fn is_word_bound(b: u8) -> bool {
    !b.is_ascii_alphanumeric() && b != b'_'
}

fn print_line(out: &mut io::Stdout, text: &str, name: &str, num: usize, cli: &GrepArgs, sn: bool) {
    if cli.count || cli.files_with {
        return;
    }
    let mut buf = String::new();
    use core::fmt::Write as _;
    if sn {
        let _ = write!(buf, "{}", name);
        if cli.line_number {
            let _ = write!(buf, ":{}", num);
        }
        buf.push(':');
    } else if cli.line_number {
        let _ = write!(buf, "{}:", num);
    }
    let _ = writeln!(buf, "{}", text);
    let _ = out.write_all(buf.as_bytes());
}

fn parse_num(s: Option<&str>) -> Option<usize> {
    s.and_then(|v| v.parse().ok())
}
