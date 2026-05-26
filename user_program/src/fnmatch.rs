//! Shell wildcard pattern matching (POSIX `fnmatch(3)`).

/// Treat `*` / `?` as not matching a `/` character.
pub const PATHNAME: u8 = 0x01;
/// Treat `\` as an ordinary character (do not escape).
pub const NOESCAPE: u8 = 0x02;
/// Case-insensitive matching.
pub const CASEFOLD: u8 = 0x04;

/// Match `string` against a shell wildcard `pattern`.
///
/// Supported metacharacters:
/// - `*` — zero or more characters (except `/` when [`PATHNAME`] is set).
/// - `?` — exactly one character (except `/` when [`PATHNAME`] is set).
/// - `[...]` — character class, e.g. `[abc]`, `[a-z]`, `[!abc]`.
/// - `\` — escape the next character (unless [`NOESCAPE`] is set).
pub fn fnmatch(pattern: &str, string: &str, flags: u8) -> bool {
    match_core(pattern.as_bytes(), string.as_bytes(), 0, 0, flags)
}

fn match_core(pat: &[u8], s: &[u8], mut pi: usize, mut si: usize, flags: u8) -> bool {
    while pi < pat.len() {
        match pat[pi] {
            b'*' => {
                pi += 1;
                if pi == pat.len() {
                    return !has_flag(flags, PATHNAME) || !s[si..].contains(&b'/');
                }
                while si <= s.len() {
                    if match_core(pat, s, pi, si, flags) {
                        return true;
                    }
                    if has_flag(flags, PATHNAME) && si < s.len() && s[si] == b'/' {
                        break;
                    }
                    si += 1;
                }
                return false;
            }
            b'?' => {
                if si >= s.len() {
                    return false;
                }
                if has_flag(flags, PATHNAME) && s[si] == b'/' {
                    return false;
                }
                pi += 1;
                si += 1;
            }
            b'[' => {
                if si >= s.len() {
                    return false;
                }
                if !match_class(pat, &mut pi, s[si], flags) {
                    return false;
                }
                si += 1;
            }
            b'\\' if !has_flag(flags, NOESCAPE) => {
                pi += 1;
                if pi >= pat.len() {
                    return si < s.len() && s[si] == b'\\';
                }
                if si >= s.len() || !byte_eq(pat[pi], s[si], flags) {
                    return false;
                }
                pi += 1;
                si += 1;
            }
            ch => {
                if si >= s.len() || !byte_eq(ch, s[si], flags) {
                    return false;
                }
                pi += 1;
                si += 1;
            }
        }
    }
    si == s.len()
}

fn match_class(pat: &[u8], pi: &mut usize, c: u8, flags: u8) -> bool {
    *pi += 1;
    if *pi >= pat.len() {
        return false;
    }
    let neg = pat[*pi] == b'!' || pat[*pi] == b'^';
    if neg {
        *pi += 1;
    }
    if *pi >= pat.len() {
        return false;
    }

    // `]` immediately after `[` or `[!` is literal.
    if pat[*pi] == b']' {
        if c == b']' {
            return !neg;
        }
        *pi += 1;
    }

    let mut matched = false;
    while *pi < pat.len() && pat[*pi] != b']' {
        let lo = pat[*pi];
        *pi += 1;

        if *pi + 1 < pat.len() && pat[*pi] == b'-' && pat[*pi + 1] != b']' {
            *pi += 1;
            let hi = pat[*pi];
            *pi += 1;
            if in_range(lo, hi, c, flags) {
                matched = true;
            }
        } else if byte_eq(lo, c, flags) {
            matched = true;
        }
    }

    if *pi >= pat.len() || pat[*pi] != b']' {
        // Unterminated — `[` is literal.
        *pi += 1;
        return c == b'[';
    }
    *pi += 1;
    matched != neg
}

fn in_range(lo: u8, hi: u8, c: u8, flags: u8) -> bool {
    if has_flag(flags, CASEFOLD) {
        let (clo, chi, cc) = (
            lo.to_ascii_lowercase(),
            hi.to_ascii_lowercase(),
            c.to_ascii_lowercase(),
        );
        cc >= clo && cc <= chi
    } else {
        c >= lo && c <= hi
    }
}

fn byte_eq(a: u8, b: u8, flags: u8) -> bool {
    if has_flag(flags, CASEFOLD) {
        a.eq_ignore_ascii_case(&b)
    } else {
        a == b
    }
}

fn has_flag(flags: u8, f: u8) -> bool {
    flags & f != 0
}
