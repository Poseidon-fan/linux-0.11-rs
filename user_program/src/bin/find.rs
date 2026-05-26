//! `find` — search for files in a directory hierarchy.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    boxed::Box,
    string::{String, ToString},
    vec::Vec,
};

use user_lib::{
    env, eprintln, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};
use user_program::fnmatch;

enum Expr {
    Name(String),
    Type(FileType),
    Print,
    Exec { cmd: String, args: Vec<String> },
    Not(Box<Expr>),
    And(Vec<Expr>),
    Or { left: Box<Expr>, right: Box<Expr> },
    True,
}

#[derive(Clone, Copy, PartialEq)]
enum FileType {
    File,
    Dir,
    Char,
    Block,
    Fifo,
    Socket,
}

impl FileType {
    fn matches(&self, meta: &fs::Metadata) -> bool {
        match self {
            FileType::File => meta.is_file(),
            FileType::Dir => meta.is_dir(),
            FileType::Char => meta.file_type().is_char_device(),
            FileType::Block => meta.file_type().is_block_device(),
            FileType::Fifo => meta.file_type().is_fifo(),
            FileType::Socket => meta.file_type().is_socket(),
        }
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut i = 0usize;
    let mut paths: Vec<String> = Vec::new();
    while i < args.len() && !is_expr_start(&args[i]) {
        paths.push(args[i].clone());
        i += 1;
    }
    if paths.is_empty() {
        paths.push(".".to_string());
    }
    let (expr, _) = parse_or(&args, i);
    for path in &paths {
        if let Ok(meta) = fs::metadata(path) {
            walk(&PathBuf::from(path.as_str()), &meta, &expr);
        } else {
            eprintln!("find: {}: No such file or directory", path);
        }
    }
    ExitCode::SUCCESS
}

fn is_expr_start(arg: &str) -> bool {
    matches!(
        arg,
        "!" | "("
            | "-not"
            | "-name"
            | "-type"
            | "-print"
            | "-exec"
            | "-true"
            | "-a"
            | "-o"
            | "-and"
            | "-or"
    ) || arg.starts_with('-')
}

// Parse: or_expr = and_expr ('-o' and_expr)*
fn parse_or(args: &[String], i: usize) -> (Expr, usize) {
    let (mut left, mut i) = parse_and(args, i);
    while i < args.len() && (args[i] == "-o" || args[i] == "-or") {
        i += 1;
        let (right, ni) = parse_and(args, i);
        i = ni;
        left = Expr::Or {
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    (left, i)
}

// Parse: and_expr = primary*
fn parse_and(args: &[String], mut i: usize) -> (Expr, usize) {
    let mut terms: Vec<Expr> = Vec::new();
    while i < args.len() && args[i] != "-o" && args[i] != "-or" && args[i] != ")" {
        let (term, ni) = parse_primary(args, i);
        i = ni;
        terms.push(term);
    }
    if terms.is_empty() {
        (Expr::True, i)
    } else if terms.len() == 1 {
        (terms.pop().unwrap(), i)
    } else {
        (Expr::And(terms), i)
    }
}

fn parse_primary(args: &[String], mut i: usize) -> (Expr, usize) {
    match args[i].as_str() {
        "!" | "-not" => {
            i += 1;
            let (inner, ni) = parse_primary(args, i);
            (Expr::Not(Box::new(inner)), ni)
        }
        "(" => {
            i += 1;
            let start = i;
            let mut depth = 1usize;
            while i < args.len() && depth > 0 {
                match args[i].as_str() {
                    "(" => depth += 1,
                    ")" => depth -= 1,
                    _ => {}
                }
                if depth > 0 {
                    i += 1;
                }
            }
            let (inner, _) = parse_or(&args[start..i], 0);
            (inner, i + 1)
        }
        "-name" => {
            i += 1;
            if i < args.len() {
                let pat = args[i].clone();
                i += 1;
                (Expr::Name(pat), i)
            } else {
                (Expr::True, i)
            }
        }
        "-type" => {
            i += 1;
            if i < args.len() {
                let ft = match args[i].as_str() {
                    "f" => FileType::File,
                    "d" => FileType::Dir,
                    "c" => FileType::Char,
                    "b" => FileType::Block,
                    "p" => FileType::Fifo,
                    "s" => FileType::Socket,
                    _ => return (Expr::True, i + 1),
                };
                i += 1;
                (Expr::Type(ft), i)
            } else {
                (Expr::True, i)
            }
        }
        "-print" => {
            i += 1;
            (Expr::Print, i)
        }
        "-exec" => {
            i += 1;
            let mut exec_args = Vec::new();
            while i < args.len() && args[i] != ";" && args[i] != "+" {
                exec_args.push(args[i].clone());
                i += 1;
            }
            if i < args.len() {
                i += 1;
            } // skip ; or +
            if exec_args.is_empty() {
                (Expr::True, i)
            } else {
                let cmd = exec_args.remove(0);
                (
                    Expr::Exec {
                        cmd,
                        args: exec_args,
                    },
                    i,
                )
            }
        }
        "-true" => {
            i += 1;
            (Expr::True, i)
        }
        _ => {
            i += 1;
            (Expr::True, i)
        }
    }
}

fn walk(path: &Path, meta: &fs::Metadata, expr: &Expr) {
    if eval(path, meta, expr) && !expr_has_action(expr) {
        let mut out = io::stdout();
        let _ = writeln!(out, "{}", path);
    }
    if meta.is_dir() {
        if let Ok(entries) = fs::read_dir(path) {
            let mut collected: Vec<PathBuf> =
                entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
            collected.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            for child_path in collected {
                if let Ok(child_meta) = fs::metadata(&child_path) {
                    walk(&child_path, &child_meta, expr);
                }
            }
        }
    }
}

fn eval(path: &Path, meta: &fs::Metadata, expr: &Expr) -> bool {
    match expr {
        Expr::True => true,
        Expr::Name(pat) => fnmatch::fnmatch(pat, path.file_name().unwrap_or(""), 0),
        Expr::Type(ft) => ft.matches(meta),
        Expr::Print => {
            let mut out = io::stdout();
            let _ = writeln!(out, "{}", path);
            true
        }
        Expr::Exec { cmd, args } => {
            let replaced: Vec<String> = args
                .iter()
                .map(|a| {
                    if a == "{}" {
                        path.as_str().to_string()
                    } else {
                        a.clone()
                    }
                })
                .collect();
            Command::new(cmd.as_str())
                .args(&replaced)
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        }
        Expr::Not(inner) => !eval(path, meta, inner),
        Expr::And(terms) => terms.iter().all(|t| eval(path, meta, t)),
        Expr::Or { left, right } => eval(path, meta, left) || eval(path, meta, right),
    }
}

fn expr_has_action(expr: &Expr) -> bool {
    match expr {
        Expr::Print | Expr::Exec { .. } => true,
        Expr::Not(i) => expr_has_action(i),
        Expr::And(ts) => ts.iter().any(expr_has_action),
        Expr::Or { left, right } => expr_has_action(left) || expr_has_action(right),
        _ => false,
    }
}
