//! `.tst` file parser.
//!
//! A `.tst` is plain text. Each non-blank, non-comment line is one
//! [`Step`]. We support five line shapes:
//!
//! | leading | meaning                                              |
//! |---------|------------------------------------------------------|
//! | `#`     | comment, ignored                                     |
//! | `> CMD` | send `CMD\r`, wait for the next prompt               |
//! | `< STR` | most recent command's output must contain `STR`      |
//! | `~ RE`  | most recent command's output must match the regex    |
//! | `! DIR` | runner directive — `send STR`, `wait-prompt`, `expect STR`, `expect-regex RE`, `timeout N`, `sleep N` |
//!
//! Blank lines are ignored.

use std::path::Path;

use anyhow::{Context, Result, anyhow};

#[derive(Debug, Clone)]
pub enum Step {
    /// Send a shell command and wait for the next prompt.
    Send(String),
    /// The most recent send's captured output must contain this substring.
    ContainsLine(String),
    /// The most recent send's captured output must match this regex.
    MatchesRegex(String),
    /// Send raw bytes (with escape decoding) — no prompt wait.
    SendRaw(String),
    /// Wait for the next prompt (used after raw-send sessions like vi).
    WaitPrompt,
    /// Block until the given substring appears in the live serial
    /// stream — useful between `! send` calls to wait for a TUI to
    /// finish redrawing before sending the next keystroke. Does not
    /// advance the cursor.
    ExpectSubstring(String),
    /// Same as `ExpectSubstring` but with a regex.
    ExpectRegex(String),
    /// Override the per-step timeout for everything that follows in
    /// this script.
    Timeout(u64),
    /// Sleep this many milliseconds (rarely needed).
    Sleep(u64),
}

#[derive(Debug, Clone)]
pub struct Script {
    pub name: String,
    pub steps: Vec<Step>,
    /// Line numbers parallel to `steps`, for error reporting.
    pub line_numbers: Vec<usize>,
}

impl Script {
    /// Loads a `.tst` from disk. `name` is the human-readable label
    /// shown in test output (e.g. `"sh/arith"`).
    pub fn load(path: &Path, name: &str) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(name, &text)
    }

    pub fn parse(name: &str, src: &str) -> Result<Self> {
        let mut steps = Vec::new();
        let mut line_numbers = Vec::new();
        for (idx, raw_line) in src.lines().enumerate() {
            let line_num = idx + 1;
            let line = raw_line.trim_end();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let step =
                parse_step(line).with_context(|| format!("{name}:{line_num}: parse error"))?;
            steps.push(step);
            line_numbers.push(line_num);
        }
        Ok(Self {
            name: name.to_string(),
            steps,
            line_numbers,
        })
    }
}

fn parse_step(line: &str) -> Result<Step> {
    let lead = line.chars().next().ok_or_else(|| anyhow!("empty line"))?;
    let rest = &line[lead.len_utf8()..];
    let rest = rest.strip_prefix(' ').unwrap_or(rest);
    match lead {
        '>' => Ok(Step::Send(rest.to_string())),
        '<' => Ok(Step::ContainsLine(rest.to_string())),
        '~' => Ok(Step::MatchesRegex(rest.to_string())),
        '!' => parse_directive(rest),
        _ => Err(anyhow!("unknown line shape (expected `# > < ~ !`)")),
    }
}

fn parse_directive(rest: &str) -> Result<Step> {
    // First whitespace separates verb from argument.
    let (verb, arg) = match rest.find(char::is_whitespace) {
        Some(i) => (&rest[..i], rest[i..].trim_start()),
        None => (rest, ""),
    };
    match verb {
        "send" => Ok(Step::SendRaw(unquote(arg).to_string())),
        "wait-prompt" => Ok(Step::WaitPrompt),
        "expect" => {
            if arg.is_empty() {
                return Err(anyhow!("`! expect STR` needs a non-empty substring"));
            }
            Ok(Step::ExpectSubstring(unquote(arg).to_string()))
        }
        "expect-regex" => {
            if arg.is_empty() {
                return Err(anyhow!("`! expect-regex RE` needs a pattern"));
            }
            Ok(Step::ExpectRegex(unquote(arg).to_string()))
        }
        "timeout" => {
            let secs: u64 = arg.parse().context("`! timeout N` expects seconds")?;
            Ok(Step::Timeout(secs))
        }
        "sleep" => {
            // Accept floats like `0.5` and convert to ms.
            let secs: f64 = arg.parse().context("`! sleep N` expects a number")?;
            if !secs.is_finite() || secs < 0.0 {
                return Err(anyhow!("`! sleep N` needs a non-negative finite number"));
            }
            Ok(Step::Sleep((secs * 1000.0) as u64))
        }
        other => Err(anyhow!("unknown directive `! {}`", other)),
    }
}

/// Strips one pair of surrounding double quotes if present. Used so
/// `! send "iHello\eZZ"` and `! send iHello` both work.
fn unquote(s: &str) -> &str {
    s.strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .unwrap_or(s)
}
