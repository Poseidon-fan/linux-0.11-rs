//! QEMU child process + serial I/O + expect / regex matching.
//!
//! Spawns `qemu-system-i386` with serial wired to the runner's
//! stdin/stdout, then drives the guest through a few primitives:
//!
//! - [`Runner::send_line`] writes one command-line worth of text and a
//!   `\r`, then waits for the next shell prompt — what `.ktest` `>` lines
//!   expand into.
//! - [`Runner::send_raw`] writes verbatim bytes (with escape decoding)
//!   without waiting for a prompt — used inside `vi` and friends.
//! - [`Runner::expect_substring`] / [`Runner::expect_regex`] poll the
//!   recent output for a match within a timeout.
//! - [`Runner::wait_prompt`] blocks until the prompt regex appears.
//!
//! Output bytes are appended to an in-memory log; the caller can dump
//! it on failure via [`Runner::log_so_far`].

use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use regex::Regex;

/// Default shell-prompt detector. Matches `# ` or `$ ` at the end of the
/// most recent line, which fits both root and unprivileged prompts.
pub const PROMPT_REGEX: &str = r"[#$] $";

/// Per-suite test driver.
pub struct Runner {
    /// QEMU child process — killed on drop.
    child: Child,
    /// Pipe into QEMU's serial input.
    stdin: ChildStdin,
    /// Everything we have read from the guest so far. Shared with the
    /// reader thread.
    log: Arc<Mutex<Vec<u8>>>,
    /// Set by the reader thread when EOF is hit.
    eof: Arc<AtomicBool>,
    /// Compiled prompt regex, configurable per-runner if the prompt
    /// shape changes.
    prompt_re: Regex,
    /// Position in `log` from which the *next* `expect` will scan.
    /// Advanced past matches so each assertion only sees its own
    /// command's output.
    cursor: usize,
}

impl Runner {
    /// Spawns QEMU. `kernel` is the floppy image, `disk` is the IDE
    /// disk image. Both paths must already exist.
    pub fn spawn(kernel: &Path, disk: &Path) -> Result<Self> {
        for (label, p) in [("kernel", kernel), ("disk", disk)] {
            if !p.exists() {
                return Err(anyhow!("{} image not found: {}", label, p.display()));
            }
        }

        let mut cmd = Command::new("qemu-system-i386");
        cmd.args([
            "-m",
            "16M",
            "-boot",
            "a",
            "-drive",
            &format!("file={},format=raw,if=floppy", kernel.display()),
            "-drive",
            &format!("file={},format=raw,if=ide,index=0", disk.display()),
            "-nographic",
            "-serial",
            "mon:stdio",
        ]);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::null());

        let mut child = cmd
            .spawn()
            .with_context(|| "failed to spawn qemu-system-i386 (is qemu installed?)")?;
        let stdin = child.stdin.take().context("qemu has no stdin")?;
        let stdout = child.stdout.take().context("qemu has no stdout")?;

        let log = Arc::new(Mutex::new(Vec::<u8>::new()));
        let eof = Arc::new(AtomicBool::new(false));
        spawn_reader(stdout, Arc::clone(&log), Arc::clone(&eof));

        let prompt_re = Regex::new(PROMPT_REGEX)?;

        Ok(Self {
            child,
            stdin,
            log,
            eof,
            prompt_re,
            cursor: 0,
        })
    }

    /// Waits for the first prompt — call once after [`Self::spawn`]
    /// before issuing any commands. The kernel takes a few seconds to
    /// boot through init.
    pub fn wait_boot(&mut self, timeout: Duration) -> Result<()> {
        self.wait_for_prompt(timeout).map(|_| ())
    }

    /// Sends `cmd` followed by `\r`, then waits for the next prompt.
    /// The returned chunk has ANSI CSI sequences stripped and the
    /// command echo (first line) removed, so callers see only the
    /// command's real output.
    pub fn send_line(&mut self, cmd: &str, timeout: Duration) -> Result<String> {
        self.stdin
            .write_all(cmd.as_bytes())
            .context("write to qemu stdin")?;
        self.stdin.write_all(b"\r").context("write CR")?;
        let raw = self.wait_for_prompt(timeout)?;
        Ok(strip_echo_and_csi(&raw))
    }

    /// Sends raw bytes, decoding `\n` `\r` `\t` `\\` `\"` `\xHH` and
    /// `\e` (Escape, `0x1b`). Does **not** wait for a prompt.
    pub fn send_raw(&mut self, escaped: &str) -> Result<()> {
        let bytes = decode_escapes(escaped)?;
        self.stdin.write_all(&bytes).context("write raw to qemu")?;
        Ok(())
    }

    /// Blocks until the prompt regex appears, then advances the cursor
    /// past it.
    pub fn wait_prompt(&mut self, timeout: Duration) -> Result<()> {
        self.wait_for_prompt(timeout)?;
        Ok(())
    }

    /// Returns once `needle` shows up in the output captured since the
    /// last `expect` / `send_line`. Does NOT advance the cursor — useful
    /// for "this output should contain X **and** Y in any order".
    pub fn expect_substring(&mut self, needle: &str, timeout: Duration) -> Result<()> {
        let escaped = regex::escape(needle);
        let re = Regex::new(&escaped).expect("regex::escape always produces a valid pattern");
        self.wait_for_match(&re, timeout, false).map(|_| ())
    }

    pub fn expect_regex(&mut self, pattern: &str, timeout: Duration) -> Result<()> {
        let re = Regex::new(pattern).with_context(|| format!("bad regex: {pattern}"))?;
        self.wait_for_match(&re, timeout, false).map(|_| ())
    }

    /// Returns the full output log accumulated so far. Used for failure
    /// reporting.
    pub fn log_so_far(&self) -> Vec<u8> {
        self.log.lock().unwrap().clone()
    }

    /// Polls the log for the regex. When found and `advance` is true,
    /// move `self.cursor` past the match. Returns the substring of the
    /// log from the previous cursor up to (and including) the match.
    fn wait_for_match(&mut self, re: &Regex, timeout: Duration, advance: bool) -> Result<String> {
        let deadline = Instant::now() + timeout;
        loop {
            {
                let log = self.log.lock().unwrap();
                if let Some(text) = std::str::from_utf8(&log[self.cursor..]).ok()
                    && let Some(m) = re.find(text)
                {
                    let absolute_end = self.cursor + m.end();
                    let chunk =
                        String::from_utf8_lossy(&log[self.cursor..absolute_end]).into_owned();
                    if advance {
                        self.cursor = absolute_end;
                    }
                    return Ok(chunk);
                }
                if self.eof.load(Ordering::Relaxed) {
                    return Err(anyhow!("qemu exited before matching `{}`", re.as_str()));
                }
            }
            if Instant::now() >= deadline {
                return Err(anyhow!(
                    "timeout waiting for `{}` (after {:?})",
                    re.as_str(),
                    timeout
                ));
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Wait for a shell prompt that belongs to completed command output.
    ///
    /// The interactive shell redraws the prompt while the runner is still
    /// typing a command, e.g. `\r\e[2K[root@linux-rs /]# w`. The generic
    /// prompt regex can briefly match the redraw when the reader thread has
    /// only received bytes through the `# ` and not the following command
    /// character yet. In shared-QEMU runs that races with `send_line()` and
    /// makes assertions inspect an empty chunk.
    ///
    /// A real post-command prompt is preceded by the line break emitted when
    /// Enter is accepted (or by a program/TTY newline), so skip prompt matches
    /// that have no `\n` between the current cursor and the match.
    fn wait_for_prompt(&mut self, timeout: Duration) -> Result<String> {
        let deadline = Instant::now() + timeout;
        loop {
            {
                let log = self.log.lock().unwrap();
                if let Ok(text) = std::str::from_utf8(&log[self.cursor..]) {
                    for m in self.prompt_re.find_iter(text) {
                        if !text[..m.start()].contains('\n') {
                            continue;
                        }
                        let absolute_end = self.cursor + m.end();
                        let chunk =
                            String::from_utf8_lossy(&log[self.cursor..absolute_end]).into_owned();
                        self.cursor = absolute_end;
                        return Ok(chunk);
                    }
                }
                if self.eof.load(Ordering::Relaxed) {
                    return Err(anyhow!("qemu exited before matching prompt"));
                }
            }
            if Instant::now() >= deadline {
                return Err(anyhow!("timeout waiting for prompt (after {:?})", timeout));
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        // Best-effort: send `exit\r` so the shell flushes, then SIGKILL
        // QEMU itself. Either may fail (already dead, broken pipe) —
        // ignore errors.
        let _ = self.stdin.write_all(b"exit\r");
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Spawns a thread that drains `stdout` into the shared log buffer.
fn spawn_reader(mut stdout: ChildStdout, log: Arc<Mutex<Vec<u8>>>, eof: Arc<AtomicBool>) {
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match stdout.read(&mut buf) {
                Ok(0) => {
                    eof.store(true, Ordering::Relaxed);
                    return;
                }
                Ok(n) => log.lock().unwrap().extend_from_slice(&buf[..n]),
                Err(_) => {
                    eof.store(true, Ordering::Relaxed);
                    return;
                }
            }
        }
    });
}

/// Translates the escape sequences understood by the `.ktest` `! send`
/// directive. Supports `\n` `\r` `\t` `\\` `\"` `\e` (ESC) and `\xHH`.
fn decode_escapes(raw: &str) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            // Push the byte form. Multibyte chars become their UTF-8
            // bytes; the runner doesn't need to interpret them.
            let mut buf = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            continue;
        }
        match chars.next() {
            Some('n') => out.push(b'\n'),
            Some('r') => out.push(b'\r'),
            Some('t') => out.push(b'\t'),
            Some('e') | Some('E') => out.push(0x1b),
            Some('\\') => out.push(b'\\'),
            Some('"') => out.push(b'"'),
            Some('\'') => out.push(b'\''),
            Some('0') => out.push(0),
            Some('x') => {
                let h1 = chars.next().ok_or_else(|| anyhow!("bad \\x escape"))?;
                let h2 = chars.next().ok_or_else(|| anyhow!("bad \\x escape"))?;
                let s: String = [h1, h2].iter().collect();
                out.push(u8::from_str_radix(&s, 16).context("bad \\x hex")?);
            }
            Some(other) => return Err(anyhow!("unknown escape `\\{}`", other)),
            None => return Err(anyhow!("trailing backslash")),
        }
    }
    Ok(out)
}

/// Convenience: where to write the suite log on failure.
pub fn log_path_for(suite: &str) -> PathBuf {
    let safe = suite.replace('/', "-");
    PathBuf::from("target/test-out").join(format!("{}.log", safe))
}

/// Removes ANSI CSI sequences (`\e[...<final>`) and the leading
/// command-echo line a line-editing shell produces. Trailing prompt
/// fragments (the next `# ` / `$ ` we matched on) are also dropped so
/// the caller sees pure command output.
fn strip_echo_and_csi(raw: &str) -> String {
    // 1. Strip CSI: ESC `[` (parameter/intermediate bytes) <final 0x40..=0x7e>.
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next(); // consume `[`
            for inner in chars.by_ref() {
                if matches!(inner, '\x40'..='\x7e') {
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }

    // 2. Drop everything up to and including the first newline — that
    //    line is the shell's echo of our command.
    let after_echo = match out.find('\n') {
        Some(p) => out[p + 1..].to_string(),
        None => String::new(),
    };

    // 3. Drop the trailing prompt fragment (the regex match is included
    //    in `raw`). We keep this tolerant: strip anything from the last
    //    `\n` onward if it looks like a prompt.
    match after_echo.rfind('\n') {
        Some(p) if looks_like_prompt(&after_echo[p + 1..]) => after_echo[..=p].to_string(),
        _ => after_echo,
    }
}

fn looks_like_prompt(tail: &str) -> bool {
    let t = tail.trim_end();
    t.ends_with('#') || t.ends_with('$')
}
