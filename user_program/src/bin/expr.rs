//! `expr` — evaluate expressions.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

use user_lib::{
    env,
    io::{self, Write},
    process::ExitCode,
};

#[user_lib::main]
fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        let _ = io::stdout().write_all(b"0\n");
        return ExitCode::from(1);
    }
    match eval(&args, 0, args.len()) {
        Ok(val) => {
            let _ = io::stdout().write_all(val.as_bytes());
            let _ = io::stdout().write_all(b"\n");
            if val != "0" && !val.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(_) => {
            let _ = io::stdout().write_all(b"0\n");
            ExitCode::from(2)
        }
    }
}

fn eval(args: &[String], start: usize, end: usize) -> Result<String, ()> {
    if start >= end {
        return Err(());
    }
    // Operator precedence:  |  <  &  <  = != < > <= >=  <  + -  <  * / %  <  :
    pipe(args, start, end)
}

fn pipe(args: &[String], s: usize, e: usize) -> Result<String, ()> {
    let (val, _) = and(args, s, e)?;
    Ok(val)
}
fn and(args: &[String], s: usize, e: usize) -> Result<(String, usize), ()> {
    let (mut left, mut i) = cmp(args, s, e)?;
    while i < e && args[i] == "&" {
        i += 1;
        let (r, ni) = cmp(args, i, e)?;
        left = if left.is_empty() || r.is_empty() || left == "0" || r == "0" {
            String::from("0")
        } else {
            left
        };
        i = ni;
    }
    Ok((left, i))
}
fn cmp(args: &[String], s: usize, e: usize) -> Result<(String, usize), ()> {
    let (mut left, mut i) = addsub(args, s, e)?;
    while i < e && matches!(args[i].as_str(), "=" | "!=" | "<" | "<=" | ">" | ">=") {
        let op = args[i].clone();
        i += 1;
        let (right, ni) = addsub(args, i, e)?;
        i = ni;
        let result = match op.as_str() {
            "=" => left == right,
            "!=" => left != right,
            "<" => left < right,
            "<=" => left <= right,
            ">" => left > right,
            ">=" => left >= right,
            _ => false,
        };
        left = if result {
            String::from("1")
        } else {
            String::from("0")
        };
    }
    Ok((left, i))
}
fn addsub(args: &[String], s: usize, e: usize) -> Result<(String, usize), ()> {
    let (mut left, mut i) = muldiv(args, s, e)?;
    while i < e && (args[i] == "+" || args[i] == "-") {
        let op = args[i].clone();
        i += 1;
        let (right, ni) = muldiv(args, i, e)?;
        i = ni;
        let l = left.parse::<i64>().unwrap_or(0);
        let r = right.parse::<i64>().unwrap_or(0);
        left = if op == "+" {
            format!("{}", l.saturating_add(r))
        } else {
            format!("{}", l.saturating_sub(r))
        };
    }
    Ok((left, i))
}
fn muldiv(args: &[String], s: usize, e: usize) -> Result<(String, usize), ()> {
    let (mut left, mut i) = match_pat(args, s, e)?;
    while i < e && (args[i] == "*" || args[i] == "/" || args[i] == "%") {
        let op = args[i].clone();
        i += 1;
        let (right, ni) = match_pat(args, i, e)?;
        i = ni;
        let l = left.parse::<i64>().unwrap_or(0);
        let r = right.parse::<i64>().unwrap_or(1);
        left = match op.as_str() {
            "*" => format!("{}", l.saturating_mul(r)),
            "/" => {
                if r == 0 {
                    return Err(());
                } else {
                    format!("{}", l / r)
                }
            }
            "%" => {
                if r == 0 {
                    return Err(());
                } else {
                    format!("{}", l % r)
                }
            }
            _ => unreachable!(),
        };
    }
    Ok((left, i))
}
// : — regex match
fn match_pat(args: &[String], s: usize, e: usize) -> Result<(String, usize), ()> {
    let (mut left, mut i) = primary(args, s, e)?;
    while i < e && args[i] == ":" {
        i += 1;
        if i >= e {
            break;
        }
        let pat = &args[i];
        i += 1;
        // Convert POSIX BRE \( \) to Perl-style ( ) for the regex crate.
        let pat_perl = pat.replace("\\(", "(").replace("\\)", ")");
        let re = regex::Regex::new(&alloc::format!("^{}", pat_perl)).map_err(|_| ())?;
        if let Some(m) = re.find(&left) {
            let matched = m.as_str();
            // If there are \(...\) groups in the pattern, return the group content
            if let Some(caps) = re.captures(&left) {
                if caps.len() > 1 {
                    left = caps
                        .get(1)
                        .map(|c| c.as_str().to_string())
                        .unwrap_or(matched.to_string());
                } else {
                    left = format!("{}", matched.len());
                }
            } else {
                left = format!("{}", matched.len());
            }
        } else {
            left = String::from("0");
            if i < e
                && matches!(
                    args[i].as_str(),
                    "&" | "|" | "=" | "!=" | "<" | ">" | "+" | "-" | "*" | "/" | "%" | ":"
                )
            {
                left = String::from("0");
            }
        }
    }
    Ok((left, i))
}
fn primary(args: &[String], s: usize, e: usize) -> Result<(String, usize), ()> {
    if s >= e {
        return Err(());
    }
    match args[s].as_str() {
        "length" => {
            if s + 1 >= e {
                Err(())
            } else {
                let val = &args[s + 1];
                Ok((format!("{}", val.len()), s + 2))
            }
        }
        "index" => {
            if s + 2 >= e {
                return Err(());
            }
            let haystack = &args[s + 1];
            let needle = &args[s + 2];
            let pos = haystack.find(needle.as_str()).map(|p| p + 1).unwrap_or(0);
            Ok((format!("{}", pos), s + 3))
        }
        "substr" => {
            if s + 3 >= e {
                return Err(());
            }
            let str_ = &args[s + 1];
            let pos: usize = args[s + 2].parse().unwrap_or(1);
            let len: usize = args[s + 3].parse().unwrap_or(0);
            let start = pos.saturating_sub(1).min(str_.len());
            let end = (start + len).min(str_.len());
            Ok((str_[start..end].to_string(), s + 4))
        }
        "match" => {
            if s + 2 >= e {
                return Err(());
            }
            let str_ = &args[s + 1];
            let pat = &args[s + 2];
            let re = regex::Regex::new(pat).map_err(|_| ())?;
            let result = if re.is_match(str_) {
                format!("{}", re.find(str_).unwrap().as_str().len())
            } else {
                String::from("0")
            };
            Ok((result, s + 3))
        }
        "(" => {
            let mut depth = 1usize;
            let mut i = s + 1;
            while i < e && depth > 0 {
                match args[i].as_str() {
                    "(" => depth += 1,
                    ")" => depth -= 1,
                    _ => {}
                }
                i += 1;
            }
            if depth != 0 {
                return Err(());
            }
            let val = eval(args, s + 1, i - 1)?;
            Ok((val, i))
        }
        "+" if s + 1 < e => primary(args, s + 1, e),
        _ => {
            let val = args[s].clone();
            Ok((val, s + 1))
        }
    }
}
