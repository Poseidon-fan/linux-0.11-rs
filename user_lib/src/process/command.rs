//! [`Command`] builder, [`Child`] handle, and [`Stdio`] configuration.
//!
//! `Stdio::piped()` exposes a pipe end to the parent. Because user processes
//! on this kernel are single-threaded, code that pipes more than one stream
//! at the same time must be careful to interleave reads and writes — a
//! parent that writes a child's `stdin` to capacity while the child has
//! filled `stdout` past the 4 KiB pipe buffer will deadlock. The same
//! caveat applies to `std::process::Command` on systems without threads;
//! the recommended pattern is to use one piped stream at a time.

use alloc::{boxed::Box, string::String, vec::Vec};
use core::{fmt, ptr};

use crate::{
    ffi::CString,
    fs::{File, OpenOptions},
    io::{Error, ErrorKind, Read, Result, Write},
    process::{ExitStatus, GroupId, UserId},
    syscall,
};

/// Describes what to do with a child process's standard I/O stream.
pub enum Stdio {
    /// The child inherits the parent's stream unchanged.
    Inherit,
    /// The stream is redirected to or from `/dev/null`.
    Null,
    /// The stream is redirected to the supplied file. The child takes
    /// ownership of the underlying fd.
    File(File),
    /// A new pipe is created. The child sees the appropriate end (read
    /// end for `stdin`, write end for `stdout`/`stderr`); the parent
    /// receives the other end on the returned [`Child`].
    Piped,
}

impl Stdio {
    /// Equivalent to [`Stdio::Inherit`].
    pub fn inherit() -> Self {
        Stdio::Inherit
    }

    /// Equivalent to [`Stdio::Null`].
    pub fn null() -> Self {
        Stdio::Null
    }

    /// Equivalent to [`Stdio::Piped`].
    pub fn piped() -> Self {
        Stdio::Piped
    }
}

impl From<File> for Stdio {
    fn from(file: File) -> Self {
        Stdio::File(file)
    }
}

impl fmt::Debug for Stdio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Stdio::Inherit => f.write_str("Inherit"),
            Stdio::Null => f.write_str("Null"),
            Stdio::File(_) => f.write_str("File(..)"),
            Stdio::Piped => f.write_str("Piped"),
        }
    }
}

// ---------------------------------------------------------------------------
// Child stream wrappers
// ---------------------------------------------------------------------------

/// A handle to a child process's standard input.
///
/// Created when [`Stdio::Piped`] is used for `stdin`. The handle is
/// consumed by writing through it; closing it (drop) sends EOF to the
/// child.
#[derive(Debug)]
pub struct ChildStdin {
    file: File,
}

impl Write for ChildStdin {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> Result<()> {
        self.file.flush()
    }
}

/// A handle to a child process's standard output.
#[derive(Debug)]
pub struct ChildStdout {
    file: File,
}

impl Read for ChildStdout {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.file.read(buf)
    }
}

/// A handle to a child process's standard error.
#[derive(Debug)]
pub struct ChildStderr {
    file: File,
}

impl Read for ChildStderr {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.file.read(buf)
    }
}

// ---------------------------------------------------------------------------
// Child
// ---------------------------------------------------------------------------

/// A representation of a running or exited child process.
///
/// Obtained from [`Command::spawn`].
#[derive(Debug)]
pub struct Child {
    pid: u32,
    /// Parent-side write end of the pipe, when [`Stdio::Piped`] was used
    /// for the child's `stdin`.
    pub stdin: Option<ChildStdin>,
    /// Parent-side read end of the pipe, when [`Stdio::Piped`] was used
    /// for the child's `stdout`.
    pub stdout: Option<ChildStdout>,
    /// Parent-side read end of the pipe, when [`Stdio::Piped`] was used
    /// for the child's `stderr`.
    pub stderr: Option<ChildStderr>,
}

impl Child {
    /// Returns the OS-assigned process identifier of the child.
    pub fn id(&self) -> u32 {
        self.pid
    }

    /// Waits for the child to exit completely, returning its exit status.
    ///
    /// Drops the parent-side `stdin` first so the child sees EOF on its
    /// standard input — this matches `std::process::Child::wait` and
    /// avoids a common deadlock where the child blocks waiting for input
    /// that will never come.
    pub fn wait(&mut self) -> Result<ExitStatus> {
        drop(self.stdin.take());

        let mut status: u32 = 0;
        loop {
            match syscall::process::waitpid(self.pid as i32, &mut status as *mut u32, 0) {
                Ok(waited) if waited == self.pid => return Ok(ExitStatus::from_raw(status)),
                Ok(_) => continue,
                Err(errno) => return Err(Error::from(errno)),
            }
        }
    }

    /// Attempts to collect the exit status of the child without blocking.
    ///
    /// Returns `Ok(None)` if the child has not yet exited.
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        const WNOHANG: u32 = 1;
        let mut status: u32 = 0;
        match syscall::process::waitpid(self.pid as i32, &mut status as *mut u32, WNOHANG) {
            Ok(0) => Ok(None),
            Ok(waited) if waited == self.pid => Ok(Some(ExitStatus::from_raw(status))),
            Ok(_) => Ok(None),
            Err(errno) => Err(Error::from(errno)),
        }
    }

    /// Sends `SIGKILL` to the child.
    pub fn kill(&mut self) -> Result<()> {
        use syscall::signal::Signal;
        syscall::signal::kill(self.pid as i32, Signal::Kill)
            .map(|_| ())
            .map_err(Error::from)
    }
}

// ---------------------------------------------------------------------------
// Command
// ---------------------------------------------------------------------------

/// A process builder, providing fine-grained control over how a new process
/// is spawned.
///
/// Mirrors [`std::process::Command`] but omits Windows-specific knobs.
pub struct Command {
    program: String,
    arg0: Option<String>,
    args: Vec<String>,
    /// User-supplied env overrides. When [`env_clear`](Self::env_clear) is
    /// `true`, this is the entire environment; otherwise it is layered on
    /// top of the parent's environment.
    env_overrides: Vec<(String, Option<String>)>,
    env_clear: bool,
    stdin: Option<Stdio>,
    stdout: Option<Stdio>,
    stderr: Option<Stdio>,
    uid: Option<UserId>,
    gid: Option<GroupId>,
    pre_exec: Option<Box<dyn FnMut() -> Result<()>>>,
}

impl Command {
    /// Creates a new [`Command`] for launching `program`.
    pub fn new<S: AsRef<str>>(program: S) -> Self {
        Self {
            program: program.as_ref().into(),
            arg0: None,
            args: Vec::new(),
            env_overrides: Vec::new(),
            env_clear: false,
            stdin: None,
            stdout: None,
            stderr: None,
            uid: None,
            gid: None,
            pre_exec: None,
        }
    }

    /// Overrides `argv[0]` for the child without changing which program
    /// file is executed.
    ///
    /// By default `argv[0]` is the same path passed to [`Command::new`].
    /// This corresponds to `os::unix::process::CommandExt::arg0` in `std`.
    pub fn arg0<S: AsRef<str>>(&mut self, arg0: S) -> &mut Self {
        self.arg0 = Some(arg0.as_ref().into());
        self
    }

    /// Adds an argument to the command.
    pub fn arg<S: AsRef<str>>(&mut self, arg: S) -> &mut Self {
        self.args.push(arg.as_ref().into());
        self
    }

    /// Adds multiple arguments to the command.
    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for arg in args {
            self.arg(arg);
        }
        self
    }

    /// Sets or overrides an environment variable for the child.
    pub fn env<K: AsRef<str>, V: AsRef<str>>(&mut self, key: K, value: V) -> &mut Self {
        self.env_overrides
            .push((key.as_ref().into(), Some(value.as_ref().into())));
        self
    }

    /// Adds or updates multiple environment variables.
    pub fn envs<I, K, V>(&mut self, vars: I) -> &mut Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        for (key, value) in vars {
            self.env(key, value);
        }
        self
    }

    /// Removes an environment variable from the child.
    pub fn env_remove<K: AsRef<str>>(&mut self, key: K) -> &mut Self {
        self.env_overrides.push((key.as_ref().into(), None));
        self
    }

    /// Clears the entire environment for the child.
    pub fn env_clear(&mut self) -> &mut Self {
        self.env_clear = true;
        self.env_overrides.clear();
        self
    }

    /// Configures the child's standard input.
    pub fn stdin<S: Into<Stdio>>(&mut self, cfg: S) -> &mut Self {
        self.stdin = Some(cfg.into());
        self
    }

    /// Configures the child's standard output.
    pub fn stdout<S: Into<Stdio>>(&mut self, cfg: S) -> &mut Self {
        self.stdout = Some(cfg.into());
        self
    }

    /// Configures the child's standard error.
    pub fn stderr<S: Into<Stdio>>(&mut self, cfg: S) -> &mut Self {
        self.stderr = Some(cfg.into());
        self
    }

    /// Sets the child process's user ID.
    ///
    /// Mirrors `std::os::unix::process::CommandExt::uid`. The child calls
    /// `setuid` after `fork` and before `execve`; a failure exits the child
    /// with status 127.
    pub fn uid(&mut self, id: UserId) -> &mut Self {
        self.uid = Some(id);
        self
    }

    /// Sets the child process's group ID.
    ///
    /// Mirrors `std::os::unix::process::CommandExt::gid`. The child calls
    /// `setgid` after `fork` and before `execve`; a failure exits the child
    /// with status 127. If both [`uid`](Self::uid) and [`gid`](Self::gid) are
    /// configured, the group ID is set first.
    pub fn gid(&mut self, id: GroupId) -> &mut Self {
        self.gid = Some(id);
        self
    }

    /// Schedules a closure to run in the child between [`fork`](syscall)
    /// and [`execve`](syscall).
    ///
    /// Useful for kernel-level steps that must happen in the new process —
    /// for example creating a new session via `setsid` or installing
    /// signal dispositions. If the closure returns an error, the child
    /// exits with status 127.
    ///
    /// Equivalent to `os::unix::process::CommandExt::pre_exec` in `std`.
    /// Unlike `std`, this method is safe: this kernel runs only one
    /// thread per process, so no async-signal-safety concerns apply.
    pub fn pre_exec<F: FnMut() -> Result<()> + 'static>(&mut self, f: F) -> &mut Self {
        self.pre_exec = Some(Box::new(f));
        self
    }

    /// Returns the program that will be launched.
    pub fn get_program(&self) -> &str {
        self.program.as_str()
    }

    /// Returns an iterator over the program's arguments.
    pub fn get_args(&self) -> impl Iterator<Item = &str> {
        self.args.iter().map(String::as_str)
    }

    /// Spawns the command, returning a [`Child`] handle.
    pub fn spawn(&mut self) -> Result<Child> {
        let program = to_cstring(self.program.as_str())?;
        let argv0 = match self.arg0.as_deref() {
            Some(s) => to_cstring(s)?,
            None => program.clone(),
        };

        let mut argv_storage: Vec<CString> = Vec::with_capacity(self.args.len() + 1);
        argv_storage.push(argv0);
        for arg in &self.args {
            argv_storage.push(to_cstring(arg.as_str())?);
        }

        let envp_storage = self.build_env()?;

        // Materialise pipes for any Piped stream BEFORE fork so both sides
        // see the file descriptors. Use Option<PipeEnds> so we know which
        // streams want piping; a None means "no pipe needed".
        let mut stdin_pipe = pipe_for(self.stdin.as_ref(), PipeRole::ChildReads)?;
        let mut stdout_pipe = match pipe_for(self.stdout.as_ref(), PipeRole::ChildWrites) {
            Ok(p) => p,
            Err(e) => {
                drop(stdin_pipe.take());
                return Err(e);
            }
        };
        let mut stderr_pipe = match pipe_for(self.stderr.as_ref(), PipeRole::ChildWrites) {
            Ok(p) => p,
            Err(e) => {
                drop(stdin_pipe.take());
                drop(stdout_pipe.take());
                return Err(e);
            }
        };

        let pid = match syscall::process::fork() {
            Ok(pid) => pid,
            Err(errno) => {
                drop(stdin_pipe.take());
                drop(stdout_pipe.take());
                drop(stderr_pipe.take());
                return Err(Error::from(errno));
            }
        };

        if pid == 0 {
            // Child path. From here on we must NOT return a `Result` —
            // returning would resume the parent's control flow. On any
            // failure we exit with 127 (the conventional "exec failure"
            // status).
            apply_stdio_in_child(0, self.stdin.as_ref(), stdin_pipe.as_ref());
            apply_stdio_in_child(1, self.stdout.as_ref(), stdout_pipe.as_ref());
            apply_stdio_in_child(2, self.stderr.as_ref(), stderr_pipe.as_ref());

            if let Some(gid) = self.gid {
                if syscall::process::setgid(gid).is_err() {
                    crate::process::exit(127);
                }
            }
            if let Some(uid) = self.uid {
                if syscall::process::setuid(uid).is_err() {
                    crate::process::exit(127);
                }
            }

            if let Some(pre_exec) = self.pre_exec.as_mut() {
                if pre_exec().is_err() {
                    crate::process::exit(127);
                }
            }

            let argv_ptrs = build_pointer_table(argv_storage.as_slice());
            let envp_ptrs = build_pointer_table(envp_storage.as_slice());

            let _ = syscall::process::execve(
                program.as_ptr().cast(),
                argv_ptrs.as_ptr(),
                envp_ptrs.as_ptr(),
            );
            crate::process::exit(127);
        }

        // Parent path: take the parent-side ends out of each pipe and
        // close the child-side ends. The remaining `PipeEnds` (only the
        // child fd left) is dropped, closing it.
        let stdin = stdin_pipe.take().map(|p| ChildStdin {
            file: p.into_parent(),
        });
        let stdout = stdout_pipe.take().map(|p| ChildStdout {
            file: p.into_parent(),
        });
        let stderr = stderr_pipe.take().map(|p| ChildStderr {
            file: p.into_parent(),
        });

        Ok(Child {
            pid,
            stdin,
            stdout,
            stderr,
        })
    }

    /// Spawns the command and waits for it to exit, returning its
    /// [`ExitStatus`].
    pub fn status(&mut self) -> Result<ExitStatus> {
        self.spawn()?.wait()
    }

    fn build_env(&self) -> Result<Vec<CString>> {
        let mut entries: Vec<(String, String)> = if self.env_clear {
            Vec::new()
        } else {
            crate::env::vars().collect()
        };

        for (key, value) in self.env_overrides.iter() {
            entries.retain(|(k, _)| k != key);
            if let Some(value) = value {
                entries.push((key.clone(), value.clone()));
            }
        }

        let mut out = Vec::with_capacity(entries.len());
        for (key, value) in entries {
            let mut joined = String::with_capacity(key.len() + value.len() + 1);
            joined.push_str(key.as_str());
            joined.push('=');
            joined.push_str(value.as_str());
            out.push(to_cstring(joined.as_str())?);
        }
        Ok(out)
    }
}

impl fmt::Debug for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let arg0: Option<&str> = self.arg0.as_deref();
        f.debug_struct("Command")
            .field("program", &self.program.as_str())
            .field("arg0", &arg0)
            .field("args", &self.args.as_slice())
            .field("env_clear", &self.env_clear)
            .field("env_overrides", &self.env_overrides.as_slice())
            .field("stdin", &self.stdin)
            .field("stdout", &self.stdout)
            .field("stderr", &self.stderr)
            .field("uid", &self.uid)
            .field("gid", &self.gid)
            .field("pre_exec", &self.pre_exec.as_ref().map(|_| "<closure>"))
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum PipeRole {
    /// Child reads from the pipe (used for child's stdin).
    ChildReads,
    /// Child writes to the pipe (used for child's stdout/stderr).
    ChildWrites,
}

/// One open pipe, oriented so we always know which fd belongs to which side.
///
/// `Drop` closes whichever ends are still held — used after fork to clean
/// up the child-side end in the parent and the parent-side end in the
/// child. [`PipeEnds::into_parent`] consumes the value and returns the
/// parent's end as a [`File`], leaving the child's end to be closed by the
/// helper that calls into it.
struct PipeEnds {
    parent_fd: Option<u32>,
    child_fd: Option<u32>,
}

impl PipeEnds {
    fn create(role: PipeRole) -> Result<Self> {
        let mut fds: [u32; 2] = [0; 2];
        syscall::fs::pipe(fds.as_mut_ptr()).map_err(Error::from)?;
        let (read_fd, write_fd) = (fds[0], fds[1]);
        let (parent_fd, child_fd) = match role {
            PipeRole::ChildReads => (write_fd, read_fd),
            PipeRole::ChildWrites => (read_fd, write_fd),
        };
        Ok(Self {
            parent_fd: Some(parent_fd),
            child_fd: Some(child_fd),
        })
    }

    /// Consumes the pipe and produces the parent-side end as a [`File`].
    /// The child-side end is closed.
    fn into_parent(mut self) -> File {
        if let Some(fd) = self.child_fd.take() {
            let _ = syscall::fs::close(fd);
        }
        let parent_fd = self.parent_fd.take().expect("parent_fd was consumed twice");
        // SAFETY: parent_fd came from a successful `pipe()` and has not been
        // closed elsewhere.
        unsafe { File::from_raw_fd(parent_fd) }
    }
}

impl Drop for PipeEnds {
    fn drop(&mut self) {
        if let Some(fd) = self.parent_fd.take() {
            let _ = syscall::fs::close(fd);
        }
        if let Some(fd) = self.child_fd.take() {
            let _ = syscall::fs::close(fd);
        }
    }
}

fn pipe_for(stdio: Option<&Stdio>, role: PipeRole) -> Result<Option<PipeEnds>> {
    match stdio {
        Some(Stdio::Piped) => Ok(Some(PipeEnds::create(role)?)),
        _ => Ok(None),
    }
}

fn to_cstring(s: &str) -> Result<CString> {
    CString::new(s).map_err(|_| Error::new(ErrorKind::InvalidInput, "argument contains a NUL byte"))
}

fn build_pointer_table(strings: &[CString]) -> Vec<*const u8> {
    let mut ptrs: Vec<*const u8> = Vec::with_capacity(strings.len() + 1);
    for s in strings {
        ptrs.push(s.as_ptr().cast());
    }
    ptrs.push(ptr::null());
    ptrs
}

/// Applies one [`Stdio`] configuration to file descriptor `fd` in the child
/// process. Failures here abort the child with code 127, mirroring the
/// conventional shell exit status for "exec failure".
fn apply_stdio_in_child(fd: u32, cfg: Option<&Stdio>, pipe: Option<&PipeEnds>) {
    let Some(cfg) = cfg else {
        return;
    };

    match cfg {
        Stdio::Inherit => {}
        Stdio::Null => {
            let mut opts = OpenOptions::new();
            opts.read(true).write(true);
            let file = match opts.open("/dev/null") {
                Ok(f) => f,
                Err(_) => crate::process::exit(127),
            };
            replace_fd_in_child(fd, file.raw_fd());
            // `file` drop closes the original fd; leave fd in the table.
            core::mem::forget(file);
        }
        Stdio::File(file) => replace_fd_in_child(fd, file.raw_fd()),
        Stdio::Piped => {
            let pipe = pipe.expect("pipe must be present for Stdio::Piped");
            // Close the parent-side end in the child, then dup2 the
            // child-side end to the target fd and close it.
            if let Some(parent_fd) = pipe.parent_fd {
                let _ = syscall::fs::close(parent_fd);
            }
            let child_fd = pipe.child_fd.expect("child_fd not yet taken");
            replace_fd_in_child(fd, child_fd);
            if child_fd != fd {
                let _ = syscall::fs::close(child_fd);
            }
        }
    }
}

fn replace_fd_in_child(target_fd: u32, source_fd: u32) {
    if target_fd == source_fd {
        return;
    }
    let _ = syscall::fs::close(target_fd);
    if syscall::fs::dup2(source_fd, target_fd).is_err() {
        crate::process::exit(127);
    }
}
