//! Built-in commands.
//!
//! Builtins run inside the shell process; that lets `cd`, `export`, `read`,
//! and the like mutate shell state in a way that an external program
//! cannot. Each builtin returns its exit status as an `i32`.

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use user_lib::{
    env, fs,
    io::{self, Write},
    syscall,
};

use crate::{exec::ExecError, state::State};

/// Returns `true` if `name` is implemented by [`dispatch`].
pub fn is_builtin(name: &str) -> bool {
    all_names().contains(&name)
}

/// Every name that [`dispatch`] knows how to handle. Used both for the
/// `is_builtin` check and by the tab-completion completer.
pub fn all_names() -> &'static [&'static str] {
    &[
        "cd", "pwd", "exit", "export", "unset", "set", "shift", "read", "echo", "eval", "exec",
        ".", "source", ":", "true", "false", "break", "continue", "return", "umask", "type",
        "wait", "command", "test", "[",
    ]
}

/// Runs a builtin by name. The returned status is what `$?` will become.
/// `eval`, `.`/`source`, `break`, `continue`, and `return` may instead
/// return a control-flow signal via [`ExecError`] for the caller to honor.
pub fn dispatch(name: &str, args: &[String], st: &mut State) -> Result<i32, ExecError> {
    match name {
        "cd" => Ok(builtin_cd(args, st)),
        "pwd" => Ok(builtin_pwd()),
        "exit" => Err(ExecError::Exit(parse_status(args.first(), st.last_status))),
        "export" => Ok(builtin_export(args, st)),
        "unset" => Ok(builtin_unset(args, st)),
        "set" => Ok(builtin_set(args, st)),
        "shift" => Ok(builtin_shift(args, st)),
        "read" => Ok(builtin_read(args, st)),
        "echo" => Ok(builtin_echo(args)),
        "eval" => builtin_eval(args, st),
        "exec" => Ok(builtin_exec(args, st)),
        "." | "source" => builtin_source(args, st),
        ":" | "true" => Ok(0),
        "false" => Ok(1),
        "break" => Err(ExecError::Break(
            parse_status(args.first(), 1).max(1) as usize
        )),
        "continue" => Err(ExecError::Continue(
            parse_status(args.first(), 1).max(1) as usize
        )),
        "return" => Err(ExecError::Return(parse_status(
            args.first(),
            st.last_status,
        ))),
        "umask" => Ok(builtin_umask(args)),
        "type" => Ok(builtin_type(args, st)),
        "wait" => Ok(builtin_wait(args)),
        "command" => Ok(builtin_command(args, st)),
        "test" => Ok(builtin_test(args, false)),
        "[" => Ok(builtin_test(args, true)),
        _ => Ok(127),
    }
}

fn builtin_cd(args: &[String], st: &mut State) -> i32 {
    let target = match args.first().map(String::as_str) {
        None | Some("") => match st.get("HOME") {
            Some(h) => h.to_string(),
            None => {
                error("cd: HOME not set");
                return 1;
            }
        },
        Some("-") => match st.get("OLDPWD") {
            Some(p) => {
                let _ = io::stdout().write_all(p.as_bytes());
                let _ = io::stdout().write_all(b"\n");
                p.to_string()
            }
            None => {
                error("cd: OLDPWD not set");
                return 1;
            }
        },
        Some(p) => p.to_string(),
    };

    let prev = env::current_dir().ok();
    if let Err(err) = fs::set_current_dir(target.as_str()) {
        error(&alloc::format!("cd: {}: {}", target, err));
        return 1;
    }
    if let Some(p) = prev {
        st.export("OLDPWD", Some(p.into_string()));
    }
    if let Ok(p) = env::current_dir() {
        st.export("PWD", Some(p.into_string()));
    }
    0
}

fn builtin_pwd() -> i32 {
    match env::current_dir() {
        Ok(p) => {
            let _ = io::stdout().write_all(p.as_str().as_bytes());
            let _ = io::stdout().write_all(b"\n");
            0
        }
        Err(err) => {
            error(&alloc::format!("pwd: {}", err));
            1
        }
    }
}

fn builtin_export(args: &[String], st: &mut State) -> i32 {
    if args.is_empty() {
        for (k, v) in st.exported_pairs() {
            let _ = writeln!(io::stdout(), "export {}={}", k, v);
        }
        return 0;
    }
    for arg in args {
        if let Some(eq) = arg.find('=') {
            let (k, v) = arg.split_at(eq);
            st.export(k, Some(v[1..].to_string()));
        } else {
            st.export(arg, None);
        }
    }
    0
}

fn builtin_unset(args: &[String], st: &mut State) -> i32 {
    let mut funcs = false;
    let mut vars = false;
    let mut names: Vec<&str> = Vec::new();
    for a in args {
        match a.as_str() {
            "-f" => funcs = true,
            "-v" => vars = true,
            other => names.push(other),
        }
    }
    if !funcs && !vars {
        vars = true;
    }
    for n in names {
        if vars {
            st.unset(n);
        }
        if funcs {
            st.undefine_function(n);
        }
    }
    0
}

fn builtin_set(args: &[String], st: &mut State) -> i32 {
    // No args → list all variables.
    if args.is_empty() {
        for (k, v) in st.all_pairs() {
            let _ = writeln!(io::stdout(), "{}={}", k, v);
        }
        return 0;
    }
    // -e / +e (errexit), -x / +x (xtrace), -u / +u (nounset), --
    let mut i = 0;
    let mut positional_set = false;
    let mut positional: Vec<String> = Vec::new();
    while i < args.len() {
        let a = &args[i];
        if a == "--" {
            i += 1;
            positional_set = true;
            while i < args.len() {
                positional.push(args[i].clone());
                i += 1;
            }
            break;
        }
        if let Some(rest) = a.strip_prefix('-') {
            for ch in rest.chars() {
                match ch {
                    'e' => st.errexit = true,
                    'x' => st.xtrace = true,
                    'u' => st.nounset = true,
                    _ => {}
                }
            }
            i += 1;
            continue;
        }
        if let Some(rest) = a.strip_prefix('+') {
            for ch in rest.chars() {
                match ch {
                    'e' => st.errexit = false,
                    'x' => st.xtrace = false,
                    'u' => st.nounset = false,
                    _ => {}
                }
            }
            i += 1;
            continue;
        }
        // Non-option: from here on, all are positional parameters.
        positional_set = true;
        while i < args.len() {
            positional.push(args[i].clone());
            i += 1;
        }
    }
    if positional_set {
        st.set_positionals(positional);
    }
    0
}

fn builtin_shift(args: &[String], st: &mut State) -> i32 {
    let n: usize = args.first().and_then(|s| s.parse().ok()).unwrap_or(1);
    if st.shift(n) { 0 } else { 1 }
}

/// Reads one line from stdin (split on IFS) and assigns the fields to the
/// named variables. The last variable receives any remaining fields joined
/// by space.
fn builtin_read(args: &[String], st: &mut State) -> i32 {
    if args.is_empty() {
        return 0;
    }
    let mut line = String::new();
    let mut stdin = io::stdin();
    let n = stdin.read_line(&mut line).unwrap_or(0);
    if n == 0 {
        return 1;
    }
    if line.ends_with('\n') {
        line.pop();
    }
    let ifs = st
        .get("IFS")
        .map(String::from)
        .unwrap_or_else(|| " \t".to_string());
    let chars: Vec<char> = ifs.chars().collect();
    let mut parts: Vec<String> = if chars.is_empty() {
        alloc::vec![line.clone()]
    } else {
        line.split(|c: char| chars.contains(&c))
            .map(String::from)
            .collect()
    };
    // Drop leading/trailing empty entries from whitespace splitting.
    while parts.first().is_some_and(|s| s.is_empty()) {
        parts.remove(0);
    }
    while parts.last().is_some_and(|s| s.is_empty()) {
        parts.pop();
    }
    let var_count = args.len();
    for (i, name) in args.iter().enumerate() {
        if i + 1 == var_count {
            // last variable gets the remainder joined.
            let rest: String = parts.drain(i..).collect::<Vec<_>>().join(" ");
            st.set(name, rest);
        } else if i < parts.len() {
            st.set(name, parts[i].clone());
        } else {
            st.set(name, String::new());
        }
    }
    0
}

fn builtin_echo(args: &[String]) -> i32 {
    let mut newline = true;
    let mut interpret = false;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "-n" {
            newline = false;
            i += 1;
            continue;
        }
        if a == "-e" {
            interpret = true;
            i += 1;
            continue;
        }
        if a == "-E" {
            interpret = false;
            i += 1;
            continue;
        }
        break;
    }
    let mut out = io::stdout();
    let mut first = true;
    for arg in &args[i..] {
        if !first {
            let _ = out.write_all(b" ");
        }
        first = false;
        if interpret {
            let _ = out.write_all(interpret_escapes(arg).as_bytes());
        } else {
            let _ = out.write_all(arg.as_bytes());
        }
    }
    if newline {
        let _ = out.write_all(b"\n");
    }
    0
}

fn interpret_escapes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            let c = bytes[i + 1];
            i += 2;
            match c {
                b'n' => out.push('\n'),
                b't' => out.push('\t'),
                b'r' => out.push('\r'),
                b'\\' => out.push('\\'),
                b'a' => out.push('\x07'),
                b'b' => out.push('\x08'),
                b'0' => out.push('\0'),
                other => {
                    out.push('\\');
                    out.push(other as char);
                }
            }
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// `eval foo bar` joins args with spaces and re-enters the parser.
fn builtin_eval(args: &[String], st: &mut State) -> Result<i32, ExecError> {
    if args.is_empty() {
        return Ok(0);
    }
    let joined = args.join(" ");
    crate::exec::run_source(&joined, st)
}

/// `exec [cmd]` — with no args, the redirections (handled by the executor
/// before reaching here) just become permanent. With a command, the shell
/// replaces itself with it via `execve`.
fn builtin_exec(args: &[String], st: &mut State) -> i32 {
    if args.is_empty() {
        return 0;
    }
    let path = match crate::exec::lookup_in_path(&args[0], st) {
        Some(p) => p,
        None => {
            error(&alloc::format!("exec: {}: not found", args[0]));
            return 127;
        }
    };
    let exec_args = match crate::exec::ExecArgs::build(&path, args, st) {
        Ok(ea) => ea,
        Err(_) => return 127,
    };
    exec_args.execve();
    error(&alloc::format!("exec: {}: cannot execute", args[0]));
    127
}

fn builtin_source(args: &[String], st: &mut State) -> Result<i32, ExecError> {
    let Some(path) = args.first() else {
        error(".: filename argument required");
        return Ok(2);
    };
    let src = match fs::read_to_string(path.as_str()) {
        Ok(s) => s,
        Err(err) => {
            error(&alloc::format!(".: {}: {}", path, err));
            return Ok(1);
        }
    };
    // `.` / `source` with extra args temporarily replaces the positional
    // parameters for the script's duration.
    let original = if args.len() > 1 {
        Some(st.replace_positionals(args[1..].to_vec()))
    } else {
        None
    };
    let r = crate::exec::run_source(&src, st);
    if let Some(p) = original {
        st.set_positionals(p);
    }
    r
}

fn builtin_umask(args: &[String]) -> i32 {
    if let Some(arg) = args.first() {
        let mask = match u32::from_str_radix(arg, 8) {
            Ok(m) => m,
            Err(_) => {
                error(&alloc::format!("umask: {}: invalid octal", arg));
                return 1;
            }
        };
        let _ = syscall::fs::umask(mask);
    } else {
        // POSIX says read current umask by setting then restoring.
        let cur = syscall::fs::umask(0).unwrap_or(0);
        let _ = syscall::fs::umask(cur);
        let _ = writeln!(io::stdout(), "{:04o}", cur);
    }
    0
}

fn builtin_type(args: &[String], st: &State) -> i32 {
    let mut had_miss = false;
    for name in args {
        if is_builtin(name) {
            let _ = writeln!(io::stdout(), "{} is a shell builtin", name);
            continue;
        }
        if st.function(name).is_some() {
            let _ = writeln!(io::stdout(), "{} is a function", name);
            continue;
        }
        if let Some(p) = crate::exec::lookup_in_path(name, st) {
            let _ = writeln!(io::stdout(), "{} is {}", name, p);
        } else {
            let _ = writeln!(io::stderr(), "{}: not found", name);
            had_miss = true;
        }
    }
    if had_miss { 1 } else { 0 }
}

/// `wait [pid]` — wait for one or all children. Without args, reap all
/// pending children until `waitpid` reports no more.
fn builtin_wait(args: &[String]) -> i32 {
    if args.is_empty() {
        loop {
            let mut status: u32 = 0;
            match syscall::process::waitpid(-1, &mut status as *mut u32, 0) {
                Ok(0) | Err(_) => return 0,
                Ok(_) => continue,
            }
        }
    }
    let mut last_status = 0;
    for a in args {
        let pid: i32 = match a.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };
        let mut status: u32 = 0;
        match syscall::process::waitpid(pid, &mut status as *mut u32, 0) {
            Ok(_) => {
                last_status = if (status & 0x7f) == 0 {
                    ((status >> 8) & 0xff) as i32
                } else {
                    128 + (status & 0x7f) as i32
                };
            }
            Err(_) => last_status = 127,
        }
    }
    last_status
}

fn builtin_command(args: &[String], st: &mut State) -> i32 {
    // command [-v] NAME — print the resolved location of NAME without
    // executing builtin/function shadowing.
    let mut i = 0;
    let mut print_kind = false;
    while i < args.len() {
        match args[i].as_str() {
            "-v" | "-V" => {
                print_kind = true;
                i += 1;
            }
            _ => break,
        }
    }
    if print_kind {
        if let Some(name) = args.get(i) {
            if is_builtin(name) {
                let _ = writeln!(io::stdout(), "{}", name);
                return 0;
            }
            if let Some(p) = crate::exec::lookup_in_path(name, st) {
                let _ = writeln!(io::stdout(), "{}", p);
                return 0;
            }
            return 1;
        }
        return 0;
    }
    // Without -v: run `args[i:]` bypassing builtins/functions — fall back
    // to whatever the executor does for ordinary commands.
    if let Some(name) = args.get(i) {
        if let Some(p) = crate::exec::lookup_in_path(name, st) {
            return crate::exec::run_external(&p, &args[i..], st).unwrap_or(127);
        }
        error(&alloc::format!("command: {}: not found", name));
        return 127;
    }
    0
}

fn parse_status(arg: Option<&String>, default: i32) -> i32 {
    arg.and_then(|s| s.parse().ok()).unwrap_or(default)
}

fn error(msg: &str) {
    let _ = writeln!(io::stderr(), "sh: {}", msg);
}

// ---------------------------------------------------------------------------
// `test` / `[`
// ---------------------------------------------------------------------------

/// POSIX `test` / `[ ... ]` — file checks and comparisons.
///
/// When invoked as `[`, the closing `]` argument is required and stripped.
fn builtin_test(args: &[String], bracket: bool) -> i32 {
    let mut args = args.to_vec();
    if bracket {
        match args.last().map(String::as_str) {
            Some("]") => {
                args.pop();
            }
            _ => {
                error("[: missing `]`");
                return 2;
            }
        }
    }
    if test_eval(&args) { 0 } else { 1 }
}

fn test_eval(args: &[String]) -> bool {
    match args.len() {
        0 => false,
        1 => !args[0].is_empty(),
        _ => {
            // Negation.
            if args[0] == "!" {
                return !test_eval(&args[1..]);
            }
            if args.len() == 2 {
                return match args[0].as_str() {
                    "-n" => !args[1].is_empty(),
                    "-z" => args[1].is_empty(),
                    op => test_unary(op, &args[1]),
                };
            }
            if args.len() == 3 {
                return test_binary(&args[0], &args[1], &args[2]);
            }
            if args.len() == 4 && (args[1] == "-a" || args[1] == "-o") {
                let l = test_eval(&args[..1]);
                let r = test_eval(&args[2..]);
                return if args[1] == "-a" { l && r } else { l || r };
            }
            false
        }
    }
}

fn test_unary(op: &str, a: &str) -> bool {
    match op {
        "-e" => fs::metadata(a).is_ok(),
        "-f" => fs::metadata(a).map(|m| m.is_file()).unwrap_or(false),
        "-d" => fs::metadata(a).map(|m| m.is_dir()).unwrap_or(false),
        "-r" => fs::metadata(a)
            .map(|m| m.mode() & 0o400 != 0)
            .unwrap_or(false),
        "-w" => fs::metadata(a)
            .map(|m| m.mode() & 0o200 != 0)
            .unwrap_or(false),
        "-x" => fs::metadata(a)
            .map(|m| m.mode() & 0o100 != 0)
            .unwrap_or(false),
        "-s" => fs::metadata(a).map(|m| !m.is_empty()).unwrap_or(false),
        _ => false,
    }
}

fn test_binary(a: &str, op: &str, b: &str) -> bool {
    match op {
        "=" | "==" => a == b,
        "!=" => a != b,
        "-eq" => parse_eq(a, b, |x, y| x == y),
        "-ne" => parse_eq(a, b, |x, y| x != y),
        "-lt" => parse_eq(a, b, |x, y| x < y),
        "-le" => parse_eq(a, b, |x, y| x <= y),
        "-gt" => parse_eq(a, b, |x, y| x > y),
        "-ge" => parse_eq(a, b, |x, y| x >= y),
        _ => false,
    }
}

fn parse_eq(a: &str, b: &str, cmp: impl Fn(i64, i64) -> bool) -> bool {
    match (a.parse::<i64>(), b.parse::<i64>()) {
        (Ok(x), Ok(y)) => cmp(x, y),
        _ => false,
    }
}
