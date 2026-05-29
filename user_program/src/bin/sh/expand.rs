//! Word expansion: tilde, parameters, command substitution, arithmetic,
//! field splitting, pathname expansion, quote removal.
//!
//! The expander walks each [`Word`] left to right, producing fragments
//! tagged with whether the surrounding context was quoted; only unquoted
//! fragments are subject to field splitting and globbing afterwards.

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use user_lib::fs;
use user_program::fnmatch;

use crate::{
    ast::{ParamOp, Seg, Word},
    lexer::LexError,
    state::State,
};

/// Expansion error: typically a syntax-level problem in a `${var:?msg}` or
/// a failed subshell command-substitution capture.
#[derive(Debug)]
pub struct ExpandError(pub String);

impl From<LexError> for ExpandError {
    fn from(err: LexError) -> Self {
        Self(err.msg)
    }
}

/// One byte of expanded output, tagged with the quote state at the
/// point where it was produced. Quoted bytes survive field splitting and
/// pathname expansion verbatim.
#[derive(Clone, Copy)]
struct Tag {
    byte: u8,
    quoted: bool,
}

/// Result of expanding one word: zero or more fully-expanded argv entries.
pub type Fields = Vec<String>;

/// Expand one word and apply field splitting + pathname expansion. Returns
/// the resulting argv fragments. An unquoted empty word produces zero
/// fields; a quoted empty word produces one empty field.
pub fn expand_word(word: &Word, st: &mut State) -> Result<Fields, ExpandError> {
    let tagged = expand_to_tagged(word, st, false)?;
    let fields = split_fields(&tagged, st.get("IFS").unwrap_or(" \t\n"));
    let mut out = Vec::new();
    for f in fields {
        out.extend(glob_expand(&f));
    }
    Ok(out)
}

/// Expand a word into a single string (no field splitting, no globbing).
/// Used for the value of a redirection target or `${var=word}`.
pub fn expand_word_unsplit(word: &Word, st: &mut State) -> Result<String, ExpandError> {
    let tagged = expand_to_tagged(word, st, false)?;
    Ok(tagged.iter().map(|t| t.byte as char).collect())
}

/// Expand one assignment value into a single string (same rules as
/// [`expand_word_unsplit`]).
pub fn expand_assignment_value(word: &Word, st: &mut State) -> Result<String, ExpandError> {
    expand_word_unsplit(word, st)
}

/// Walks a word and returns a tagged byte stream describing the
/// pre-splitting result. Inner regions inside `"..."` propagate
/// `quoted = true`.
fn expand_to_tagged(
    word: &Word,
    st: &mut State,
    parent_quoted: bool,
) -> Result<Vec<Tag>, ExpandError> {
    let mut out = Vec::new();
    let mut first = true;
    for seg in &word.0 {
        let was_first = first;
        first = false;
        expand_seg(seg, st, parent_quoted, was_first, &mut out)?;
    }
    Ok(out)
}

fn expand_seg(
    seg: &Seg,
    st: &mut State,
    quoted: bool,
    word_start: bool,
    out: &mut Vec<Tag>,
) -> Result<(), ExpandError> {
    match seg {
        Seg::Lit(s) => push_str(out, s, quoted),
        Seg::SQuoted(s) => push_str(out, s, true),
        Seg::DQuoted(inner) => {
            for s in inner {
                expand_seg(s, st, true, false, out)?;
            }
        }
        Seg::Tilde(name) => {
            if !word_start {
                push_str(out, "~", quoted);
                push_str(out, name, quoted);
                return Ok(());
            }
            let value = if name.is_empty() {
                st.get("HOME").map(String::from).unwrap_or_default()
            } else {
                // No /etc/passwd parsing — just preserve `~name` if we
                // can't resolve it.
                let mut buf = String::from("~");
                buf.push_str(name);
                buf
            };
            push_str(out, &value, quoted);
        }
        Seg::Param {
            name,
            op,
            arg,
            length,
        } => {
            let value = lookup_param(name, st);
            let expanded = match (op, arg) {
                (None, _) => value.unwrap_or_default(),
                (Some(op), Some(arg)) => apply_param_op(*op, name, value, arg, st)?,
                (Some(_), None) => value.unwrap_or_default(),
            };
            if *length {
                push_str(out, &expanded.chars().count().to_string(), quoted);
            } else if name == "@" && quoted {
                // "$@" — expand each positional parameter as its own field
                // by injecting a non-IFS separator. Field splitting later
                // honours it specially.
                let params = st.positionals().to_vec();
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        out.push(Tag {
                            byte: 0x01,
                            quoted: false,
                        });
                    }
                    push_str(out, p, true);
                }
            } else {
                push_str(out, &expanded, quoted);
            }
        }
        Seg::CmdSub(src) => {
            let captured = run_command_substitution(src, st)?;
            push_str(out, &captured, quoted);
        }
        Seg::Arith(src) => {
            let val = eval_arith(src, st)?;
            push_str(out, &val.to_string(), quoted);
        }
    }
    Ok(())
}

fn push_str(out: &mut Vec<Tag>, s: &str, quoted: bool) {
    for b in s.bytes() {
        out.push(Tag { byte: b, quoted });
    }
}

/// Returns the value of a special or named parameter without applying any
/// `${...}` operator.
fn lookup_param(name: &str, st: &State) -> Option<String> {
    match name {
        "?" => Some(st.last_status.to_string()),
        "$" => Some(user_lib::process::id().to_string()),
        "!" => Some(st.last_bg_pid.to_string()),
        "#" => Some(st.positional_count().to_string()),
        "@" => Some(st.positionals().join(" ")),
        "*" => Some(st.positionals().join(" ")),
        n if n.bytes().all(|c| c.is_ascii_digit()) && !n.is_empty() => {
            let i: usize = n.parse().unwrap_or(0);
            Some(st.positional(i).to_string())
        }
        n => st.get(n).map(String::from),
    }
}

fn apply_param_op(
    op: ParamOp,
    name: &str,
    value: Option<String>,
    arg: &Word,
    st: &mut State,
) -> Result<String, ExpandError> {
    let (use_default, assign, error_on_miss, invert) = match op {
        ParamOp::DashUnset => (value.is_none(), false, false, false),
        ParamOp::DashNull => (
            value.as_deref().is_none_or(str::is_empty),
            false,
            false,
            false,
        ),
        ParamOp::EqUnset => (value.is_none(), true, false, false),
        ParamOp::EqNull => (
            value.as_deref().is_none_or(str::is_empty),
            true,
            false,
            false,
        ),
        ParamOp::PlusUnset => (value.is_none(), false, false, true),
        ParamOp::PlusNull => (
            value.as_deref().is_none_or(str::is_empty),
            false,
            false,
            true,
        ),
        ParamOp::QmarkUnset => (value.is_none(), false, true, false),
        ParamOp::QmarkNull => (
            value.as_deref().is_none_or(str::is_empty),
            false,
            true,
            false,
        ),
    };

    if invert {
        // `${var+word}` / `${var:+word}`: use `word` only when condition
        // is FALSE (i.e., when the variable IS set / non-empty).
        if !use_default {
            return expand_word_unsplit(arg, st);
        }
        return Ok(String::new());
    }
    if use_default {
        let new = expand_word_unsplit(arg, st)?;
        if error_on_miss {
            return Err(ExpandError(if new.is_empty() {
                alloc::format!("{}: parameter null or not set", name)
            } else {
                alloc::format!("{}: {}", name, new)
            }));
        }
        if assign {
            st.set(name, new.clone());
        }
        Ok(new)
    } else {
        Ok(value.unwrap_or_default())
    }
}

/// Runs `src` as a subshell command and returns its standard output with
/// trailing newlines stripped (the POSIX command-substitution rule).
fn run_command_substitution(src: &str, st: &mut State) -> Result<String, ExpandError> {
    let mut parser = crate::parser::Parser::new(src)?;
    let cmd = parser.parse_program()?;
    let captured = crate::exec::capture_subshell(&cmd, st);
    let mut s = captured.unwrap_or_default();
    while s.ends_with('\n') {
        s.pop();
    }
    Ok(s)
}

/// Splits a tagged byte stream into fields using the characters in `ifs`.
/// Whitespace IFS characters (` \t\n` in the default IFS) coalesce; other
/// IFS characters delimit individual empty fields. Tagged bytes whose
/// `quoted` flag is set are never split, except for the synthetic
/// `0x01` byte injected by `"$@"`.
fn split_fields(tagged: &[Tag], ifs: &str) -> Fields {
    if tagged.is_empty() {
        return Vec::new();
    }
    let ws_ifs: Vec<u8> = ifs.bytes().filter(u8::is_ascii_whitespace).collect();
    let non_ws_ifs: Vec<u8> = ifs.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let is_ws = |b: u8| ws_ifs.contains(&b);
    let is_nonws = |b: u8| non_ws_ifs.contains(&b);

    let mut fields: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut have_field = false;

    let mut i = 0;
    while i < tagged.len() {
        let t = tagged[i];
        // Synthetic "$@" separator: always splits even when surrounding
        // context is quoted, never produces empty fields by itself.
        if t.byte == 0x01 && !t.quoted {
            if have_field {
                fields.push(core::mem::take(&mut current));
                have_field = false;
            }
            i += 1;
            continue;
        }
        if !t.quoted && is_ws(t.byte) {
            if have_field {
                fields.push(core::mem::take(&mut current));
                have_field = false;
            }
            // skip following whitespace IFS
            while i + 1 < tagged.len() && !tagged[i + 1].quoted && is_ws(tagged[i + 1].byte) {
                i += 1;
            }
            i += 1;
            continue;
        }
        if !t.quoted && is_nonws(t.byte) {
            // non-whitespace IFS always terminates exactly one field, even
            // if the next char is also IFS.
            fields.push(core::mem::take(&mut current));
            have_field = false;
            i += 1;
            continue;
        }
        // Quoted glob metacharacters must survive pathname expansion
        // unchanged. Backslash-escape them here; `glob_expand` treats
        // `\*` / `\?` / `\[` as literal and `unescape_globs` peels the
        // escape off again.
        if t.quoted && matches!(t.byte, b'*' | b'?' | b'[' | b'\\') {
            current.push('\\');
        }
        current.push(t.byte as char);
        have_field = true;
        i += 1;
    }
    if have_field {
        fields.push(current);
    }
    // A wholly-quoted empty input still produces one empty field.
    if fields.is_empty() && tagged.iter().any(|t| t.quoted) {
        fields.push(String::new());
    }
    fields
}

/// Performs pathname expansion: if `field` contains any unquoted glob
/// metacharacters, expand it against the filesystem; otherwise return it
/// as-is. We use a simple heuristic over the raw field string:
/// a backslash-escaped meta (`\*`, `\?`, `\[`) doesn't trigger globbing
/// and is unescaped in place; an unescaped meta does.
fn glob_expand(field: &str) -> Vec<String> {
    let mut has_unescaped_meta = false;
    let mut iter = field.bytes().peekable();
    while let Some(b) = iter.next() {
        if b == b'\\' {
            iter.next();
            continue;
        }
        if matches!(b, b'*' | b'?' | b'[') {
            has_unescaped_meta = true;
            break;
        }
    }
    if !has_unescaped_meta {
        return alloc::vec![unescape_globs(field)];
    }
    let matches = match_glob(field);
    if matches.is_empty() {
        alloc::vec![unescape_globs(field)]
    } else {
        matches
    }
}

/// Strip backslash before glob metacharacters (`\*` → `*`, `\?` → `?`,
/// `\[` → `[`, `\\` → `\`). Leaves every other backslash sequence alone.
fn unescape_globs(field: &str) -> String {
    let mut out = String::with_capacity(field.len());
    let bytes = field.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 1 < bytes.len()
            && matches!(bytes[i + 1], b'*' | b'?' | b'[' | b'\\')
        {
            out.push(bytes[i + 1] as char);
            i += 2;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

/// Splits `pattern` by `/` and walks the filesystem, matching each segment
/// against the entries of its parent directory. Supports leading `/` for
/// absolute patterns.
fn match_glob(pattern: &str) -> Vec<String> {
    let absolute = pattern.starts_with('/');
    let parts: Vec<&str> = pattern.split('/').filter(|p| !p.is_empty()).collect();
    let mut current: Vec<String> = if absolute {
        alloc::vec!["/".to_string()]
    } else {
        alloc::vec![String::new()]
    };
    for (depth, part) in parts.iter().enumerate() {
        let is_last = depth + 1 == parts.len();
        let mut next: Vec<String> = Vec::new();
        let has_meta = part.bytes().any(|b| matches!(b, b'*' | b'?' | b'['));
        for base in &current {
            let dir = if base.is_empty() {
                ".".to_string()
            } else {
                base.clone()
            };
            if !has_meta {
                let joined = join(base, part);
                if is_last
                    || fs::metadata(joined.as_str())
                        .map(|m| m.is_dir())
                        .unwrap_or(false)
                {
                    next.push(joined);
                }
                continue;
            }
            let Ok(rd) = fs::read_dir(dir.as_str()) else {
                continue;
            };
            let mut entries: Vec<String> = Vec::new();
            for e in rd.flatten() {
                let name = e.file_name();
                if name.starts_with('.') && !part.starts_with('.') {
                    continue;
                }
                if fnmatch::fnmatch(part, &name, 0) {
                    entries.push(name);
                }
            }
            entries.sort();
            for name in entries {
                let joined = join(base, &name);
                if is_last
                    || fs::metadata(joined.as_str())
                        .map(|m| m.is_dir())
                        .unwrap_or(false)
                {
                    next.push(joined);
                }
            }
        }
        current = next;
        if current.is_empty() {
            return Vec::new();
        }
    }
    current
}

fn join(base: &str, name: &str) -> String {
    if base.is_empty() {
        return name.to_string();
    }
    if base == "/" {
        return alloc::format!("/{}", name);
    }
    if base.ends_with('/') {
        return alloc::format!("{}{}", base, name);
    }
    alloc::format!("{}/{}", base, name)
}

// ---------------------------------------------------------------------------
// Arithmetic
// ---------------------------------------------------------------------------

/// Evaluates an arithmetic expression as in `$((expr))`, returning a signed
/// 64-bit integer. Supports the operators in the precedence table inside
/// [`arith::parse`].
fn eval_arith(src: &str, st: &mut State) -> Result<i64, ExpandError> {
    // Arithmetic operands may themselves contain parameter or command
    // substitutions ("$1 + 2", "$(echo 3) * 4"). Re-parse the source as a
    // double-quoted word so those expansions happen before tokenization.
    let expanded = expand_arith_source(src, st)?;
    let tokens: Vec<arith::Tok> = arith::tokenize(&expanded, st)?;
    let mut p = arith::Parser::new(&tokens);
    let v = p.parse()?;
    if !p.is_done() {
        return Err(ExpandError("arithmetic: trailing tokens".to_string()));
    }
    Ok(v)
}

/// Performs `$param` / `$(...)` expansion on the textual contents of an
/// arithmetic expression. Quotes and IFS are not relevant here.
fn expand_arith_source(src: &str, st: &mut State) -> Result<String, ExpandError> {
    let wrapped = alloc::format!("\"{}\"", src.replace('"', "\\\""));
    let mut lex = crate::lexer::Lexer::new(&wrapped);
    let tok = lex.next_token().map_err(|e| ExpandError(e.msg))?;
    let word = match tok {
        crate::lexer::Token::Word(w) => w,
        _ => return Ok(src.to_string()),
    };
    expand_word_unsplit(&word, st)
}

mod arith {
    use alloc::{string::ToString, vec::Vec};

    use super::{ExpandError, State};

    #[derive(Clone, Debug)]
    pub enum Tok {
        Num(i64),
        Plus,
        Minus,
        Star,
        Slash,
        Percent,
        LParen,
        RParen,
        Eq,
        Neq,
        Lt,
        Gt,
        Le,
        Ge,
        And,
        Or,
        Not,
    }

    pub fn tokenize(src: &str, st: &mut State) -> Result<Vec<Tok>, ExpandError> {
        let bytes = src.as_bytes();
        let mut i = 0;
        let mut out = Vec::new();
        while i < bytes.len() {
            let c = bytes[i];
            if c.is_ascii_whitespace() {
                i += 1;
                continue;
            }
            if c.is_ascii_digit() {
                let start = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                let n: i64 = src[start..i]
                    .parse()
                    .map_err(|_| ExpandError("arithmetic: bad number".to_string()))?;
                out.push(Tok::Num(n));
                continue;
            }
            if c.is_ascii_alphabetic() || c == b'_' {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                let name = &src[start..i];
                let n = st
                    .get(name)
                    .and_then(|v| v.parse::<i64>().ok())
                    .unwrap_or(0);
                out.push(Tok::Num(n));
                continue;
            }
            match c {
                b'+' => out.push(Tok::Plus),
                b'-' => out.push(Tok::Minus),
                b'*' => out.push(Tok::Star),
                b'/' => out.push(Tok::Slash),
                b'%' => out.push(Tok::Percent),
                b'(' => out.push(Tok::LParen),
                b')' => out.push(Tok::RParen),
                b'=' if i + 1 < bytes.len() && bytes[i + 1] == b'=' => {
                    out.push(Tok::Eq);
                    i += 1;
                }
                b'!' if i + 1 < bytes.len() && bytes[i + 1] == b'=' => {
                    out.push(Tok::Neq);
                    i += 1;
                }
                b'<' if i + 1 < bytes.len() && bytes[i + 1] == b'=' => {
                    out.push(Tok::Le);
                    i += 1;
                }
                b'>' if i + 1 < bytes.len() && bytes[i + 1] == b'=' => {
                    out.push(Tok::Ge);
                    i += 1;
                }
                b'<' => out.push(Tok::Lt),
                b'>' => out.push(Tok::Gt),
                b'&' if i + 1 < bytes.len() && bytes[i + 1] == b'&' => {
                    out.push(Tok::And);
                    i += 1;
                }
                b'|' if i + 1 < bytes.len() && bytes[i + 1] == b'|' => {
                    out.push(Tok::Or);
                    i += 1;
                }
                b'!' => out.push(Tok::Not),
                _ => {
                    return Err(ExpandError(alloc::format!(
                        "arithmetic: bad char `{}`",
                        c as char
                    )));
                }
            }
            i += 1;
        }
        Ok(out)
    }

    pub struct Parser<'a> {
        toks: &'a [Tok],
        pos: usize,
    }

    impl<'a> Parser<'a> {
        pub fn new(toks: &'a [Tok]) -> Self {
            Self { toks, pos: 0 }
        }
        pub fn is_done(&self) -> bool {
            self.pos >= self.toks.len()
        }
        pub fn parse(&mut self) -> Result<i64, ExpandError> {
            self.or()
        }
        fn or(&mut self) -> Result<i64, ExpandError> {
            let mut left = self.and()?;
            while matches!(self.peek(), Some(Tok::Or)) {
                self.pos += 1;
                let r = self.and()?;
                left = bool_to_int((left != 0) || (r != 0));
            }
            Ok(left)
        }
        fn and(&mut self) -> Result<i64, ExpandError> {
            let mut left = self.cmp()?;
            while matches!(self.peek(), Some(Tok::And)) {
                self.pos += 1;
                let r = self.cmp()?;
                left = bool_to_int((left != 0) && (r != 0));
            }
            Ok(left)
        }
        fn cmp(&mut self) -> Result<i64, ExpandError> {
            let mut left = self.addsub()?;
            loop {
                let op = match self.peek() {
                    Some(Tok::Eq) => 0,
                    Some(Tok::Neq) => 1,
                    Some(Tok::Lt) => 2,
                    Some(Tok::Le) => 3,
                    Some(Tok::Gt) => 4,
                    Some(Tok::Ge) => 5,
                    _ => break,
                };
                self.pos += 1;
                let r = self.addsub()?;
                left = bool_to_int(match op {
                    0 => left == r,
                    1 => left != r,
                    2 => left < r,
                    3 => left <= r,
                    4 => left > r,
                    5 => left >= r,
                    _ => unreachable!(),
                });
            }
            Ok(left)
        }
        fn addsub(&mut self) -> Result<i64, ExpandError> {
            let mut left = self.muldiv()?;
            loop {
                match self.peek() {
                    Some(Tok::Plus) => {
                        self.pos += 1;
                        left = left.wrapping_add(self.muldiv()?);
                    }
                    Some(Tok::Minus) => {
                        self.pos += 1;
                        left = left.wrapping_sub(self.muldiv()?);
                    }
                    _ => break,
                }
            }
            Ok(left)
        }
        fn muldiv(&mut self) -> Result<i64, ExpandError> {
            let mut left = self.unary()?;
            loop {
                match self.peek() {
                    Some(Tok::Star) => {
                        self.pos += 1;
                        left = left.wrapping_mul(self.unary()?);
                    }
                    Some(Tok::Slash) => {
                        self.pos += 1;
                        let r = self.unary()?;
                        if r == 0 {
                            return Err(ExpandError("arithmetic: division by zero".to_string()));
                        }
                        left = left.wrapping_div(r);
                    }
                    Some(Tok::Percent) => {
                        self.pos += 1;
                        let r = self.unary()?;
                        if r == 0 {
                            return Err(ExpandError("arithmetic: division by zero".to_string()));
                        }
                        left = left.wrapping_rem(r);
                    }
                    _ => break,
                }
            }
            Ok(left)
        }
        fn unary(&mut self) -> Result<i64, ExpandError> {
            match self.peek() {
                Some(Tok::Plus) => {
                    self.pos += 1;
                    self.unary()
                }
                Some(Tok::Minus) => {
                    self.pos += 1;
                    Ok(-self.unary()?)
                }
                Some(Tok::Not) => {
                    self.pos += 1;
                    Ok(bool_to_int(self.unary()? == 0))
                }
                _ => self.primary(),
            }
        }
        fn primary(&mut self) -> Result<i64, ExpandError> {
            match self.peek() {
                Some(Tok::Num(n)) => {
                    let n = *n;
                    self.pos += 1;
                    Ok(n)
                }
                Some(Tok::LParen) => {
                    self.pos += 1;
                    let v = self.parse()?;
                    if !matches!(self.peek(), Some(Tok::RParen)) {
                        return Err(ExpandError("arithmetic: missing ')'".to_string()));
                    }
                    self.pos += 1;
                    Ok(v)
                }
                _ => Err(ExpandError("arithmetic: expected number".to_string())),
            }
        }
        fn peek(&self) -> Option<&Tok> {
            self.toks.get(self.pos)
        }
    }

    fn bool_to_int(b: bool) -> i64 {
        if b { 1 } else { 0 }
    }
}
