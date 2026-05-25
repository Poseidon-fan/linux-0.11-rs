//! `tr` — translate or delete characters from standard input.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::String, vec::Vec};

use anyhow::{Result, bail};
use user_lib::io::{self, Read, Write};
use user_program::cli::cli_args;

cli_args! {
    /// Translate, squeeze, or delete characters from standard input,
    /// writing the result to standard output.
    pub struct TrArgs {
        /// Delete characters in SET1, do not translate.
        pub delete:     bool       = ["-d", "--delete"],
        /// Replace each input sequence of a repeated character listed in the
        /// last specified SET with a single occurrence of that character.
        pub squeeze:    bool       = ["-s", "--squeeze-repeats"],
        /// Use the complement of SET1.
        pub complement: bool       = ["-c", "-C", "--complement"],
        /// First truncate SET1 to length of SET2.
        pub truncate:   bool       = ["-t", "--truncate-set1"],
        /// Character sets (SET1 and optionally SET2).
        pub sets:       Vec<String> = [..] @ "SET",
    }
}

#[user_lib::main]
fn main() -> Result<()> {
    let cli = TrArgs::parse_env_or_exit();

    let set1_raw = cli.sets.first().map(String::as_str).unwrap_or("");
    let set2_raw = cli.sets.get(1).map(String::as_str);

    if set1_raw.is_empty() {
        bail!("missing operand");
    }

    let mut set1 = expand_set(set1_raw)?;
    let mut set2 = if let Some(s) = set2_raw {
        Some(expand_set(s)?)
    } else {
        None
    };

    let mode = decide_mode(&cli, set2.is_some())?;

    if let Mode::Translate = mode {
        let s2 = set2.as_mut().expect("translate mode without SET2");
        if cli.truncate {
            s2.truncate(set1.len());
        }
        // POSIX: if SET2 is shorter than SET1, the last char of SET2 is
        // repeated to pad. (GNU tr default behavior.)
        if s2.is_empty() {
            bail!("when not deleting, SET2 must be non-empty");
        }
        let pad = *s2.last().unwrap();
        while s2.len() < set1.len() {
            s2.push(pad);
        }
    }

    let translate_table = match &mode {
        Mode::Translate => Some(build_translate_table(
            &set1,
            set2.as_deref().unwrap(),
            cli.complement,
        )),
        _ => None,
    };

    let in_set1 = if cli.complement {
        invert_membership(&set1)
    } else {
        membership(&set1)
    };
    let in_squeeze_set: Option<[bool; 256]> = if cli.squeeze {
        // POSIX: with -s alone uses SET1; with translate uses SET2.
        let sq_src = match &mode {
            Mode::Translate => set2.as_deref().unwrap(),
            _ => set1.as_slice(),
        };
        // Note: for translate, the squeeze test is on the *output* bytes —
        // characters from SET2 after translation. The membership is built
        // over SET2's raw bytes; we apply squeeze post-translate below.
        Some(membership(sq_src))
    } else {
        None
    };

    let mut stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut in_buf = [0u8; 1024];
    let mut out_buf: Vec<u8> = Vec::with_capacity(1024);
    let mut last_emitted: Option<u8> = None;

    loop {
        let n = stdin.read(&mut in_buf)?;
        if n == 0 {
            break;
        }
        out_buf.clear();
        for &b in &in_buf[..n] {
            match &mode {
                Mode::Delete => {
                    if in_set1[b as usize] {
                        continue;
                    }
                    push_with_squeeze(&mut out_buf, b, &mut last_emitted, &in_squeeze_set);
                }
                Mode::SqueezeOnly => {
                    push_with_squeeze(&mut out_buf, b, &mut last_emitted, &in_squeeze_set);
                }
                Mode::Translate => {
                    let table = translate_table.as_ref().unwrap();
                    let mapped = table[b as usize];
                    push_with_squeeze(&mut out_buf, mapped, &mut last_emitted, &in_squeeze_set);
                }
            }
        }
        stdout.write_all(&out_buf)?;
    }

    Ok(())
}

enum Mode {
    Translate,
    Delete,
    SqueezeOnly,
}

fn decide_mode(cli: &TrArgs, has_set2: bool) -> Result<Mode> {
    match (cli.delete, cli.squeeze, has_set2) {
        (true, false, true) => bail!("extra operand when deleting without squeeze"),
        (true, _, _) => Ok(Mode::Delete),
        (false, _, true) => Ok(Mode::Translate),
        (false, true, false) => Ok(Mode::SqueezeOnly),
        (false, false, false) => {
            bail!("need two SETs to translate, or -d to delete, or -s to squeeze")
        }
    }
}

/// Append `byte` to `out`, suppressing it when squeeze mode is on and the
/// previous output byte was the same and is in the squeeze set.
fn push_with_squeeze(
    out: &mut Vec<u8>,
    byte: u8,
    last: &mut Option<u8>,
    squeeze_set: &Option<[bool; 256]>,
) {
    if let Some(set) = squeeze_set {
        if *last == Some(byte) && set[byte as usize] {
            return;
        }
    }
    out.push(byte);
    *last = Some(byte);
}

/// Builds a 256-entry membership table: `[b as usize]` is true iff byte `b`
/// appears in `set`.
fn membership(set: &[u8]) -> [bool; 256] {
    let mut table = [false; 256];
    for &b in set {
        table[b as usize] = true;
    }
    table
}

fn invert_membership(set: &[u8]) -> [bool; 256] {
    let mut table = membership(set);
    for slot in table.iter_mut() {
        *slot = !*slot;
    }
    table
}

/// `table[b as usize]` is the output byte for input `b`. Bytes outside
/// SET1 pass through unchanged.
fn build_translate_table(set1: &[u8], set2: &[u8], complement: bool) -> [u8; 256] {
    let mut table = [0u8; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        *slot = i as u8;
    }
    if complement {
        // For -c with translate, all bytes NOT in SET1 are mapped to the
        // last byte of SET2 (POSIX rule).
        let in_set1 = membership(set1);
        let pad = *set2.last().unwrap();
        for (i, slot) in table.iter_mut().enumerate() {
            if !in_set1[i] {
                *slot = pad;
            }
        }
    } else {
        for (i, &b) in set1.iter().enumerate() {
            if let Some(&mapped) = set2.get(i) {
                table[b as usize] = mapped;
            }
        }
    }
    table
}

// ---------------------------------------------------------------------------
// SET parsing
// ---------------------------------------------------------------------------

fn expand_set(input: &str) -> Result<Vec<u8>> {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        // Character class: `[:class:]`
        if bytes[i] == b'[' && i + 1 < bytes.len() && bytes[i + 1] == b':' {
            if let Some(end) = find_subsequence(&bytes[i + 2..], b":]") {
                let name = core::str::from_utf8(&bytes[i + 2..i + 2 + end])
                    .map_err(|_| anyhow::anyhow!("invalid character class"))?;
                expand_class(name, &mut out)?;
                i = i + 2 + end + 2;
                continue;
            }
        }

        let (ch, next) = read_char(bytes, i)?;

        // Range: `a-z`
        if next < bytes.len() && bytes[next] == b'-' && next + 1 < bytes.len() {
            let (end_ch, after) = read_char(bytes, next + 1)?;
            if ch > end_ch {
                bail!(
                    "invalid range: {}-{} (start > end)",
                    ch as char,
                    end_ch as char
                );
            }
            for c in ch..=end_ch {
                out.push(c);
            }
            i = after;
            continue;
        }

        out.push(ch);
        i = next;
    }
    Ok(out)
}

/// Returns `(byte, next_index)` after parsing one (possibly escaped) byte.
fn read_char(bytes: &[u8], i: usize) -> Result<(u8, usize)> {
    if bytes[i] != b'\\' {
        return Ok((bytes[i], i + 1));
    }
    if i + 1 >= bytes.len() {
        return Ok((b'\\', i + 1));
    }
    let esc = bytes[i + 1];
    let ch = match esc {
        b'\\' => b'\\',
        b'a' => b'\x07',
        b'b' => b'\x08',
        b'f' => b'\x0c',
        b'n' => b'\n',
        b'r' => b'\r',
        b't' => b'\t',
        b'v' => b'\x0b',
        b'0'..=b'7' => {
            // Up to 3 octal digits total, starting with this one.
            let mut value: u32 = 0;
            let mut count = 0;
            let mut j = i + 1;
            while count < 3 && j < bytes.len() && (b'0'..=b'7').contains(&bytes[j]) {
                value = value * 8 + (bytes[j] - b'0') as u32;
                j += 1;
                count += 1;
            }
            return Ok(((value & 0xff) as u8, j));
        }
        other => other,
    };
    Ok((ch, i + 2))
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn expand_class(name: &str, out: &mut Vec<u8>) -> Result<()> {
    let push_range = |out: &mut Vec<u8>, lo: u8, hi: u8| {
        for c in lo..=hi {
            out.push(c);
        }
    };
    match name {
        "alpha" => {
            push_range(out, b'A', b'Z');
            push_range(out, b'a', b'z');
        }
        "upper" => push_range(out, b'A', b'Z'),
        "lower" => push_range(out, b'a', b'z'),
        "digit" => push_range(out, b'0', b'9'),
        "alnum" => {
            push_range(out, b'A', b'Z');
            push_range(out, b'a', b'z');
            push_range(out, b'0', b'9');
        }
        "xdigit" => {
            push_range(out, b'0', b'9');
            push_range(out, b'A', b'F');
            push_range(out, b'a', b'f');
        }
        "space" => {
            out.extend_from_slice(b" \t\n\r\x0b\x0c");
        }
        "blank" => {
            out.extend_from_slice(b" \t");
        }
        "cntrl" => {
            push_range(out, 0, 0x1f);
            out.push(0x7f);
        }
        "print" => push_range(out, b' ', b'~'),
        "graph" => push_range(out, b'!', b'~'),
        "punct" => {
            for c in b'!'..=b'~' {
                if !c.is_ascii_alphanumeric() {
                    out.push(c);
                }
            }
        }
        other => bail!("invalid character class: [:{}:]", other),
    }
    Ok(())
}

#[allow(dead_code)]
fn _retain_string() -> String {
    String::new()
}
