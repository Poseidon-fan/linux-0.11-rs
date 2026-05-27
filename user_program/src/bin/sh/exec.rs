//! Command execution.
//!
//! The executor walks the AST, expanding words and dispatching to either an
//! in-process builtin or a forked child running `execve`. Pipelines wire up
//! `pipe(2)` chains; control-flow forms (`if`, `while`, `for`, …) interpret
//! recursively.

use alloc::{
    ffi::CString,
    string::{String, ToString},
    vec::Vec,
};

use user_lib::{
    fs::{self, OpenOptions},
    io::{self, Write},
    syscall::{
        self,
        fs::{AccessMode, OpenFlags, OpenOptions as FsOpenOptions},
    },
};

use crate::{
    ast::{AndOrOp, Cmd, Redir, RedirOp, RedirTarget, Sep, SimpleCmd},
    builtin, expand,
    parser::Parser,
    state::State,
};

/// A non-trivial completion of execution: either a control-flow signal
/// from a builtin (`break`, `continue`, `return`, `exit`) or a fatal
/// expansion/parse error.
#[derive(Debug)]
pub enum ExecError {
    /// `exit STATUS` — leave the shell with this status.
    Exit(i32),
    /// `return STATUS` — leave the enclosing function or sourced script.
    Return(i32),
    /// `break N` — exit `N` enclosing loops.
    Break(usize),
    /// `continue N` — continue at the `N`-th enclosing loop.
    Continue(usize),
    /// A non-recoverable error message that should be printed to stderr.
    Fatal(String),
}

impl From<expand::ExpandError> for ExecError {
    fn from(err: expand::ExpandError) -> Self {
        ExecError::Fatal(err.0)
    }
}

impl From<crate::lexer::LexError> for ExecError {
    fn from(err: crate::lexer::LexError) -> Self {
        ExecError::Fatal(err.msg)
    }
}

/// Parses `src` and runs the resulting program against `st`. The returned
/// `i32` is the final `$?` value (also reflected in `st.last_status`).
pub fn run_source(src: &str, st: &mut State) -> Result<i32, ExecError> {
    let mut parser = Parser::new(src)?;
    let prog = parser.parse_program()?;
    run_cmd(&prog, st)
}

/// Runs one AST node, propagating control-flow errors upward.
pub fn run_cmd(cmd: &Cmd, st: &mut State) -> Result<i32, ExecError> {
    let status = match cmd {
        Cmd::Empty => 0,
        Cmd::Simple(s) => run_simple(s, st)?,
        Cmd::Pipeline { negated, parts } => {
            let s = run_pipeline(parts, st)?;
            if *negated {
                if s == 0 { 1 } else { 0 }
            } else {
                s
            }
        }
        Cmd::AndOr { left, op, right } => {
            let l = run_cmd(left, st)?;
            st.last_status = l;
            match op {
                AndOrOp::And if l == 0 => run_cmd(right, st)?,
                AndOrOp::Or if l != 0 => run_cmd(right, st)?,
                _ => l,
            }
        }
        Cmd::List(items) => {
            let mut last = 0;
            for (item, sep) in items {
                match sep {
                    Sep::Seq => {
                        last = run_cmd(item, st)?;
                        st.last_status = last;
                        if st.errexit && last != 0 {
                            return Err(ExecError::Exit(last));
                        }
                    }
                    Sep::Bg => {
                        let pid = run_in_background(item, st)?;
                        st.last_bg_pid = pid;
                        last = 0;
                    }
                }
            }
            last
        }
        Cmd::Subshell { body, redirs } => run_subshell(body, redirs, st)?,
        Cmd::Group { body, redirs } => with_redirs(redirs, st, |st| run_cmd(body, st))?,
        Cmd::If {
            cond,
            then,
            elifs,
            els,
            redirs,
        } => with_redirs(redirs, st, |st| {
            let c = run_cmd(cond, st)?;
            st.last_status = c;
            if c == 0 {
                return run_cmd(then, st);
            }
            for (c, b) in elifs {
                let v = run_cmd(c, st)?;
                st.last_status = v;
                if v == 0 {
                    return run_cmd(b, st);
                }
            }
            if let Some(e) = els {
                return run_cmd(e, st);
            }
            Ok(0)
        })?,
        Cmd::While {
            cond,
            body,
            until,
            redirs,
        } => with_redirs(redirs, st, |st| {
            let mut last = 0;
            loop {
                let c = run_cmd(cond, st)?;
                st.last_status = c;
                let go = if *until { c != 0 } else { c == 0 };
                if !go {
                    break;
                }
                match run_cmd(body, st) {
                    Ok(s) => last = s,
                    Err(ExecError::Break(1)) => break,
                    Err(ExecError::Break(n)) => return Err(ExecError::Break(n - 1)),
                    Err(ExecError::Continue(1)) => continue,
                    Err(ExecError::Continue(n)) => return Err(ExecError::Continue(n - 1)),
                    Err(other) => return Err(other),
                }
            }
            Ok(last)
        })?,
        Cmd::For {
            var,
            words,
            body,
            redirs,
        } => with_redirs(redirs, st, |st| {
            let items: Vec<String> = match words {
                Some(ws) => {
                    let mut out = Vec::new();
                    for w in ws {
                        out.extend(expand::expand_word(w, st)?);
                    }
                    out
                }
                None => st.positionals().to_vec(),
            };
            let mut last = 0;
            for v in items {
                st.set(var, v);
                match run_cmd(body, st) {
                    Ok(s) => last = s,
                    Err(ExecError::Break(1)) => break,
                    Err(ExecError::Break(n)) => return Err(ExecError::Break(n - 1)),
                    Err(ExecError::Continue(1)) => continue,
                    Err(ExecError::Continue(n)) => return Err(ExecError::Continue(n - 1)),
                    Err(other) => return Err(other),
                }
            }
            Ok(last)
        })?,
        Cmd::FuncDef { name, body } => {
            st.define_function(name.clone(), (**body).clone());
            0
        }
    };
    st.last_status = status;
    Ok(status)
}

// ---------------------------------------------------------------------------
// Simple commands
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Simple commands
// ---------------------------------------------------------------------------

fn run_simple(cmd: &SimpleCmd, st: &mut State) -> Result<i32, ExecError> {
    // Pre-expand assignments. They become temporary if the command also
    // has words; otherwise they apply to the shell.
    let mut tmp_env: Vec<(String, String, bool)> = Vec::new(); // (name, value, was_set)

    // Expand argv.
    let mut argv: Vec<String> = Vec::new();
    for w in &cmd.words {
        argv.extend(expand::expand_word(w, st)?);
    }

    if argv.is_empty() {
        // Assignments only — apply to the shell.
        for a in &cmd.assigns {
            let v = expand::expand_assignment_value(&a.value, st)?;
            st.set(&a.name, v);
        }
        // Open and immediately close redirections, like POSIX dictates.
        let _ = with_redirs(&cmd.redirs, st, |_| Ok(0))?;
        return Ok(0);
    }

    // Apply assignments as temporary.
    for a in &cmd.assigns {
        let v = expand::expand_assignment_value(&a.value, st)?;
        let prev = st.get(&a.name).map(String::from);
        tmp_env.push((a.name.clone(), v.clone(), prev.is_some()));
        st.set(&a.name, v);
        // POSIX: for non-builtin commands, the assignment is exported for
        // the duration of that command. Mark exported.
        st.export(&a.name, None);
    }

    // Restore tmp_env on the way out for shell builtins. We capture the
    // previous values now.
    let original: Vec<(String, Option<String>)> = tmp_env
        .iter()
        .map(|(name, _, _)| (name.clone(), st.get(name).map(String::from)))
        .collect();
    let _ = original;

    if st.xtrace {
        let _ = writeln!(io::stderr(), "+ {}", argv.join(" "));
    }

    let name = argv[0].clone();

    // 1. Functions take precedence over builtins for non-special names.
    if !is_special_builtin(&name) {
        if let Some(body) = st.function(&name).cloned() {
            let prev = st.replace_positionals(argv[1..].to_vec());
            let result = with_redirs(&cmd.redirs, st, |st| match run_cmd(&body, st) {
                Ok(s) => Ok(s),
                Err(ExecError::Return(s)) => Ok(s),
                Err(other) => Err(other),
            });
            st.set_positionals(prev);
            return result;
        }
    }

    // 2. Builtin?
    if builtin::is_builtin(&name) {
        let r = with_redirs(&cmd.redirs, st, |st| {
            builtin::dispatch(&name, &argv[1..], st)
        })?;
        // Revert temporary assignments. POSIX says non-special builtins
        // do not retain them; special builtins (like `:`/`set`) do.
        if !is_special_builtin(&name) {
            for (k, v, was) in tmp_env.iter().rev() {
                if *was {
                    st.set(k, v.clone());
                } else {
                    st.unset(k);
                }
            }
        }
        return Ok(r);
    }

    // 3. External command.
    let r = with_redirs(&cmd.redirs, st, |st| {
        run_external_lookup(&name, &argv, st).map_err(ExecError::Fatal)
    });

    // Revert temporary assignments (external commands never retain them).
    for (k, v, was) in tmp_env.iter().rev() {
        if *was {
            st.set(k, v.clone());
        } else {
            st.unset(k);
        }
    }

    r
}

fn is_special_builtin(name: &str) -> bool {
    matches!(
        name,
        ":" | "."
            | "source"
            | "break"
            | "continue"
            | "eval"
            | "exec"
            | "exit"
            | "export"
            | "readonly"
            | "return"
            | "set"
            | "shift"
            | "unset"
    )
}

fn run_external_lookup(name: &str, argv: &[String], st: &State) -> Result<i32, String> {
    let path = lookup_in_path(name, st).ok_or_else(|| alloc::format!("{}: not found", name))?;
    run_external(&path, argv, st)
}

/// Fork + execve `path` with `argv`. Returns the child's exit status.
pub fn run_external(path: &str, argv: &[String], st: &State) -> Result<i32, String> {
    let exec_args = ExecArgs::build(path, argv, st)?;
    let pid = syscall::process::fork().map_err(|e| alloc::format!("fork: {:?}", e))?;
    if pid == 0 {
        exec_args.execve();
        let _ = writeln!(io::stderr(), "sh: {}: cannot execute", path);
        user_lib::process::exit(127);
    }
    let mut status: u32 = 0;
    syscall::process::waitpid(pid as i32, &mut status as *mut u32, 0)
        .map_err(|e| alloc::format!("waitpid: {:?}", e))?;
    Ok(decode_status(status))
}

/// Marshalled `execve` arguments: the program path plus argv/envp pointer
/// tables. Owning the backing `CString` storage on the same value keeps the
/// `*const u8` entries in [`ExecArgs::execve`] valid for the call.
pub struct ExecArgs {
    program: CString,
    // The Vecs below are kept alive solely to back `argv_ptrs` / `envp_ptrs`;
    // suppress the unused-field lint with leading underscores would lose the
    // documentation, so we mark them `#[allow]` instead.
    #[allow(dead_code)]
    argv_storage: Vec<CString>,
    #[allow(dead_code)]
    envp_storage: Vec<CString>,
    argv_ptrs: Vec<*const u8>,
    envp_ptrs: Vec<*const u8>,
}

impl ExecArgs {
    /// Builds the marshalled tables from `path`, `argv`, and the exported
    /// environment in `st`. Entries containing interior NUL bytes are
    /// skipped silently — they cannot be represented in the C ABI.
    pub fn build(path: &str, argv: &[String], st: &State) -> Result<Self, String> {
        let program = CString::new(path).map_err(|_| "path contains NUL".to_string())?;
        let argv_storage: Vec<CString> = argv
            .iter()
            .filter_map(|a| CString::new(a.as_str()).ok())
            .collect();
        let envp_storage: Vec<CString> = st
            .exported_pairs()
            .into_iter()
            .filter_map(|(k, v)| CString::new(alloc::format!("{}={}", k, v)).ok())
            .collect();
        let argv_ptrs = pointer_table(&argv_storage);
        let envp_ptrs = pointer_table(&envp_storage);
        Ok(Self {
            program,
            argv_storage,
            envp_storage,
            argv_ptrs,
            envp_ptrs,
        })
    }

    /// Issues the `execve` syscall. On success this never returns; on
    /// failure the caller should print an error and exit the child.
    pub fn execve(&self) {
        let _ = syscall::process::execve(
            self.program.as_ptr().cast(),
            self.argv_ptrs.as_ptr(),
            self.envp_ptrs.as_ptr(),
        );
    }
}

fn pointer_table(strings: &[CString]) -> Vec<*const u8> {
    strings
        .iter()
        .map(|s| s.as_ptr() as *const u8)
        .chain(core::iter::once(core::ptr::null()))
        .collect()
}

/// Resolve a command name to a path. Names containing `/` are returned
/// unchanged (POSIX behavior); otherwise we try each `PATH` element.
pub fn lookup_in_path(name: &str, st: &State) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    if name.contains('/') {
        return if fs::metadata(name).map(|m| m.is_file()).unwrap_or(false) {
            Some(name.to_string())
        } else {
            None
        };
    }
    let path = st.get("PATH").unwrap_or("/bin:/usr/bin");
    for dir in path.split(':') {
        let dir = if dir.is_empty() { "." } else { dir };
        let candidate = if dir.ends_with('/') {
            alloc::format!("{}{}", dir, name)
        } else {
            alloc::format!("{}/{}", dir, name)
        };
        if fs::metadata(candidate.as_str())
            .map(|m| m.is_file())
            .unwrap_or(false)
        {
            return Some(candidate);
        }
    }
    None
}

fn decode_status(status: u32) -> i32 {
    let sig = status & 0x7f;
    if sig == 0 {
        ((status >> 8) & 0xff) as i32
    } else {
        128 + sig as i32
    }
}

// ---------------------------------------------------------------------------
// Pipelines
// ---------------------------------------------------------------------------

fn run_pipeline(parts: &[Cmd], st: &mut State) -> Result<i32, ExecError> {
    if parts.len() == 1 {
        return run_cmd(&parts[0], st);
    }
    // Build a chain of pipes and fork one child per stage.
    let mut prev_read: Option<u32> = None;
    let mut pids: Vec<u32> = Vec::with_capacity(parts.len());
    for (i, part) in parts.iter().enumerate() {
        let (read_end, write_end) = if i + 1 < parts.len() {
            let mut fds: [u32; 2] = [0; 2];
            syscall::fs::pipe(fds.as_mut_ptr())
                .map_err(|e| ExecError::Fatal(alloc::format!("pipe: {:?}", e)))?;
            (Some(fds[0]), Some(fds[1]))
        } else {
            (None, None)
        };
        let pid = syscall::process::fork()
            .map_err(|e| ExecError::Fatal(alloc::format!("fork: {:?}", e)))?;
        if pid == 0 {
            // Child: wire stdin from prev_read, stdout to write_end.
            if let Some(r) = prev_read {
                let _ = syscall::fs::dup2(r, 0);
                let _ = syscall::fs::close(r);
            }
            if let Some(w) = write_end {
                let _ = syscall::fs::dup2(w, 1);
                let _ = syscall::fs::close(w);
            }
            if let Some(r) = read_end {
                let _ = syscall::fs::close(r);
            }
            // Run this stage and exit with its status.
            let mut local_st = clone_for_subshell(st);
            let s = match run_cmd(part, &mut local_st) {
                Ok(s) => s,
                Err(ExecError::Exit(s)) => s,
                Err(_) => 1,
            };
            user_lib::process::exit(s);
        }
        // Parent: close ends we no longer need.
        if let Some(r) = prev_read {
            let _ = syscall::fs::close(r);
        }
        if let Some(w) = write_end {
            let _ = syscall::fs::close(w);
        }
        prev_read = read_end;
        pids.push(pid);
    }
    // Wait for all stages; final status is the last one.
    let mut last = 0;
    for pid in pids {
        let mut status: u32 = 0;
        if syscall::process::waitpid(pid as i32, &mut status as *mut u32, 0).is_ok() {
            last = decode_status(status);
        }
    }
    Ok(last)
}

// ---------------------------------------------------------------------------
// Background, subshell, command substitution
// ---------------------------------------------------------------------------

fn run_in_background(cmd: &Cmd, st: &State) -> Result<u32, ExecError> {
    let pid =
        syscall::process::fork().map_err(|e| ExecError::Fatal(alloc::format!("fork: {:?}", e)))?;
    if pid == 0 {
        // Detach from terminal-foreground process group.
        let _ = syscall::process::setpgid(0, 0);
        // Redirect stdin from /dev/null to avoid stealing input.
        if let Ok(f) = OpenOptions::new().read(true).open("/dev/null") {
            // Note: File doesn't expose raw_fd publicly outside user_lib;
            // we fall back to a syscall-level open.
            drop(f);
        }
        let _ = syscall::fs::close(0);
        let _ = syscall::fs::open(
            c"/dev/null".as_ptr().cast(),
            OpenFlags::from_raw(AccessMode::ReadOnly as u32),
            0,
        );
        let mut local_st = clone_for_subshell(st);
        let s = match run_cmd(cmd, &mut local_st) {
            Ok(s) => s,
            Err(ExecError::Exit(s)) => s,
            Err(_) => 1,
        };
        user_lib::process::exit(s);
    }
    Ok(pid)
}

fn run_subshell(body: &Cmd, redirs: &[Redir], st: &mut State) -> Result<i32, ExecError> {
    let pid =
        syscall::process::fork().map_err(|e| ExecError::Fatal(alloc::format!("fork: {:?}", e)))?;
    if pid == 0 {
        let mut local_st = clone_for_subshell(st);
        let r = with_redirs(redirs, &mut local_st, |st| run_cmd(body, st));
        let s = match r {
            Ok(s) => s,
            Err(ExecError::Exit(s)) => s,
            Err(_) => 1,
        };
        user_lib::process::exit(s);
    }
    let mut status: u32 = 0;
    syscall::process::waitpid(pid as i32, &mut status as *mut u32, 0)
        .map_err(|e| ExecError::Fatal(alloc::format!("waitpid: {:?}", e)))?;
    Ok(decode_status(status))
}

/// Fork, run `cmd` with stdout piped back, and return the captured bytes
/// as a UTF-8 string. Used by `$(...)` substitution.
pub fn capture_subshell(cmd: &Cmd, st: &State) -> Option<String> {
    let mut fds: [u32; 2] = [0; 2];
    if syscall::fs::pipe(fds.as_mut_ptr()).is_err() {
        return None;
    }
    let (rd, wr) = (fds[0], fds[1]);
    let pid = syscall::process::fork().ok()?;
    if pid == 0 {
        let _ = syscall::fs::close(rd);
        let _ = syscall::fs::dup2(wr, 1);
        let _ = syscall::fs::close(wr);
        let mut local_st = clone_for_subshell(st);
        let s = match run_cmd(cmd, &mut local_st) {
            Ok(s) => s,
            Err(ExecError::Exit(s)) => s,
            Err(_) => 1,
        };
        user_lib::process::exit(s);
    }
    let _ = syscall::fs::close(wr);
    let mut out = String::new();
    let mut buf = [0u8; 1024];
    loop {
        let n = syscall::fs::read(rd, buf.as_mut_ptr(), buf.len() as i32).unwrap_or(0);
        if n == 0 {
            break;
        }
        for &b in &buf[..n as usize] {
            out.push(b as char);
        }
    }
    let _ = syscall::fs::close(rd);
    let mut status: u32 = 0;
    let _ = syscall::process::waitpid(pid as i32, &mut status as *mut u32, 0);
    Some(out)
}

fn clone_for_subshell(st: &State) -> State {
    // The forked child inherits the variable table by value (Linux 0.11
    // copy-on-write fork) — but `State` itself is per-process. We make a
    // duplicate so the child's mutations don't escape via static state.
    let mut env: Vec<(String, String)> = st.all_pairs();
    let mut new = State::from_env(st.arg0.clone(), st.positionals().to_vec());
    // Overwrite with exact parent state, including non-exported variables.
    for (k, v) in env.drain(..) {
        new.set(&k, v);
    }
    for (k, _) in st.exported_pairs() {
        new.export(&k, None);
    }
    for (name, body) in st.all_functions() {
        new.define_function(name.to_string(), body.clone());
    }
    new.last_status = st.last_status;
    new.last_bg_pid = st.last_bg_pid;
    new.errexit = st.errexit;
    new.xtrace = st.xtrace;
    new.nounset = st.nounset;
    new
}

// ---------------------------------------------------------------------------
// Redirections
// ---------------------------------------------------------------------------

/// Applies `redirs`, runs `f`, then restores the original fds. Returns the
/// result of `f` (or a fatal error if redirection setup fails).
pub fn with_redirs<F, T>(redirs: &[Redir], st: &mut State, f: F) -> Result<T, ExecError>
where F: FnOnce(&mut State) -> Result<T, ExecError> {
    // For each redirection, we `dup` the existing fd to save it, then
    // perform the action. On the way out we restore by `dup2`-ing back.
    let mut saved: Vec<(i32, u32)> = Vec::new(); // (target_fd, saved_fd)
    let result = (|| -> Result<T, ExecError> {
        for r in redirs {
            apply_redir(r, st, &mut saved)?;
        }
        f(st)
    })();
    // Always restore, even on error.
    for (target, saved) in saved.into_iter().rev() {
        let _ = syscall::fs::dup2(saved, target as u32);
        let _ = syscall::fs::close(saved);
    }
    result
}

fn apply_redir(r: &Redir, st: &mut State, saved: &mut Vec<(i32, u32)>) -> Result<(), ExecError> {
    let default_fd = match r.op {
        RedirOp::In | RedirOp::DupIn => 0,
        RedirOp::Out | RedirOp::Append | RedirOp::DupOut | RedirOp::Clobber => 1,
        RedirOp::OutBoth => 1,
    };
    let target_fd = r.fd.unwrap_or(default_fd);

    // Save the current target_fd by dup'ing it so we can restore later.
    if let Ok(s) = syscall::fs::dup(target_fd as u32) {
        saved.push((target_fd, s));
    } // If target was not open, dup fails — nothing to restore for it.

    let source_word = match &r.target {
        RedirTarget::File(w) | RedirTarget::Fd(w) => w,
    };
    let word_str = expand::expand_word_unsplit(source_word, st)?;

    match r.op {
        RedirOp::In => open_path_to(
            &word_str,
            target_fd,
            AccessMode::ReadOnly,
            FsOpenOptions::empty(),
        )?,
        RedirOp::Out | RedirOp::Clobber => open_path_to(
            &word_str,
            target_fd,
            AccessMode::WriteOnly,
            FsOpenOptions::CREATE | FsOpenOptions::TRUNCATE,
        )?,
        RedirOp::Append => open_path_to(
            &word_str,
            target_fd,
            AccessMode::WriteOnly,
            FsOpenOptions::CREATE | FsOpenOptions::APPEND,
        )?,
        RedirOp::OutBoth => {
            // &> FILE — open file, dup to fd 1 and fd 2.
            let new_fd = open_path(
                &word_str,
                AccessMode::WriteOnly,
                FsOpenOptions::CREATE | FsOpenOptions::TRUNCATE,
            )?;
            // Save fd 2 too.
            if let Ok(s) = syscall::fs::dup(2) {
                saved.push((2, s));
            }
            let _ = syscall::fs::dup2(new_fd, 1);
            let _ = syscall::fs::dup2(new_fd, 2);
            let _ = syscall::fs::close(new_fd);
        }
        RedirOp::DupOut | RedirOp::DupIn => {
            if word_str == "-" {
                let _ = syscall::fs::close(target_fd as u32);
            } else {
                let src: i32 = word_str
                    .parse()
                    .map_err(|_| ExecError::Fatal(alloc::format!("bad fd: {}", word_str)))?;
                if syscall::fs::dup2(src as u32, target_fd as u32).is_err() {
                    return Err(ExecError::Fatal(alloc::format!(
                        "{}: bad fd {}",
                        target_fd,
                        src
                    )));
                }
            }
        }
    }
    Ok(())
}

fn open_path(path: &str, mode: AccessMode, options: FsOpenOptions) -> Result<u32, ExecError> {
    let cpath = CString::new(path).map_err(|_| ExecError::Fatal("path has NUL".to_string()))?;
    syscall::fs::open(cpath.as_ptr().cast(), OpenFlags::new(mode, options), 0o644)
        .map_err(|e| ExecError::Fatal(alloc::format!("cannot open {}: {:?}", path, e)))
}

fn open_path_to(
    path: &str,
    target_fd: i32,
    mode: AccessMode,
    options: FsOpenOptions,
) -> Result<(), ExecError> {
    let new_fd = open_path(path, mode, options)?;
    let _ = syscall::fs::dup2(new_fd, target_fd as u32);
    let _ = syscall::fs::close(new_fd);
    Ok(())
}
