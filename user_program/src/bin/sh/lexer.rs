//! Lexer.
//!
//! Splits the raw source into a stream of [`Token`]s while building up
//! [`Word`](crate::ast::Word) values that preserve quoting information.
//!
//! Shell tokenization is context-sensitive (quote state, brace nesting,
//! here-doc collection), so this is hand-written rather than table-driven.

use alloc::{
    boxed::Box,
    string::{String, ToString},
    vec::Vec,
};
use core::fmt;

use crate::ast::{ParamOp, Seg, Word};

/// One token produced by the lexer.
#[derive(Clone, Debug)]
pub enum Token {
    /// A word — a sequence of one or more segments, after lexical quoting
    /// has been resolved but before expansion has taken place.
    Word(Word),
    /// Statement separator. Equivalent to `;` in most contexts but kept
    /// distinct so the parser can recognise where blank lines end blocks.
    Newline,
    /// `;`.
    Semi,
    /// `&` — run preceding command in background.
    Amp,
    /// `&&`.
    AndAnd,
    /// `||`.
    OrOr,
    /// `|`.
    Pipe,
    /// `(`.
    LParen,
    /// `)`.
    RParen,
    /// `<`.
    Less,
    /// `>`.
    Greater,
    /// `>>`.
    DGreater,
    /// `>|`.
    Clobber,
    /// `<&`.
    LessAnd,
    /// `>&`.
    GreaterAnd,
    /// `&>`.
    AmpGreater,
    /// An [IO_NUMBER] — a non-negative integer that immediately precedes a
    /// redirection operator.
    ///
    /// [IO_NUMBER]: https://pubs.opengroup.org/onlinepubs/9699919799/utilities/V3_chap02.html
    IoNumber(i32),
    /// End of input.
    Eof,
}

/// An error from the lexer or parser.
#[derive(Debug)]
pub struct LexError {
    pub msg: String,
    /// `true` if the input ended in the middle of a quote / `$(` / `${`
    /// region. Used by the interactive REPL to issue a continuation prompt.
    pub incomplete: bool,
}

impl LexError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self {
            msg: msg.into(),
            incomplete: false,
        }
    }
    pub fn incomplete(msg: impl Into<String>) -> Self {
        Self {
            msg: msg.into(),
            incomplete: true,
        }
    }
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.msg)
    }
}

/// The lexer state. Construct with [`Lexer::new`] then drive with
/// [`Lexer::next_token`].
pub struct Lexer<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    /// Creates a lexer over `src`.
    pub fn new(src: &'a str) -> Self {
        Self {
            bytes: src.as_bytes(),
            pos: 0,
        }
    }

    /// Pulls the next token from the stream.
    ///
    /// Newlines yield [`Token::Newline`] (commands can span multiple lines
    /// after operators or inside compound constructs; the parser decides).
    pub fn next_token(&mut self) -> Result<Token, LexError> {
        self.skip_blanks_and_comments();
        if self.pos >= self.bytes.len() {
            return Ok(Token::Eof);
        }
        let b = self.bytes[self.pos];

        // Operator characters.
        match b {
            b'\n' => {
                self.pos += 1;
                return Ok(Token::Newline);
            }
            b';' => {
                self.pos += 1;
                return Ok(Token::Semi);
            }
            b'(' => {
                self.pos += 1;
                return Ok(Token::LParen);
            }
            b')' => {
                self.pos += 1;
                return Ok(Token::RParen);
            }
            b'|' => {
                self.pos += 1;
                if self.peek() == Some(b'|') {
                    self.pos += 1;
                    return Ok(Token::OrOr);
                }
                return Ok(Token::Pipe);
            }
            b'&' => {
                self.pos += 1;
                match self.peek() {
                    Some(b'&') => {
                        self.pos += 1;
                        return Ok(Token::AndAnd);
                    }
                    Some(b'>') => {
                        self.pos += 1;
                        return Ok(Token::AmpGreater);
                    }
                    _ => return Ok(Token::Amp),
                }
            }
            b'<' => {
                self.pos += 1;
                if self.peek() == Some(b'&') {
                    self.pos += 1;
                    return Ok(Token::LessAnd);
                }
                return Ok(Token::Less);
            }
            b'>' => {
                self.pos += 1;
                match self.peek() {
                    Some(b'>') => {
                        self.pos += 1;
                        return Ok(Token::DGreater);
                    }
                    Some(b'&') => {
                        self.pos += 1;
                        return Ok(Token::GreaterAnd);
                    }
                    Some(b'|') => {
                        self.pos += 1;
                        return Ok(Token::Clobber);
                    }
                    _ => return Ok(Token::Greater),
                }
            }
            _ => {}
        }

        // Otherwise: a word, possibly preceded by an IO number that
        // immediately abuts a redirection operator.
        let start = self.pos;
        let word = self.read_word()?;
        if word.0.len() == 1 {
            if let Seg::Lit(text) = &word.0[0] {
                if text.bytes().all(|c| c.is_ascii_digit()) && !text.is_empty() {
                    // Peek ahead — if a redirection operator follows with no
                    // intervening blank, treat the digits as an IO_NUMBER.
                    if matches!(self.peek(), Some(b'<') | Some(b'>'))
                        && (start..self.pos).len() == text.len()
                    {
                        if let Ok(n) = text.parse::<i32>() {
                            return Ok(Token::IoNumber(n));
                        }
                    }
                }
            }
        }
        Ok(Token::Word(word))
    }

    /// Reads one word starting at the current position. Handles quoting,
    /// nested `$(...)`, `${...}`, `$((...))` and inline operators.
    fn read_word(&mut self) -> Result<Word, LexError> {
        let mut segs: Vec<Seg> = Vec::new();
        let mut lit = String::new();
        let mut at_word_start = true;

        // Tilde expansion is only recognised when `~` is the very first
        // character of the word (we don't model `:`-separated assignment
        // value tildes here — that is a rarely used POSIX corner).
        if self.peek() == Some(b'~') {
            self.pos += 1;
            let mut name = String::new();
            while let Some(c) = self.peek() {
                if c.is_ascii_alphanumeric() || c == b'_' || c == b'.' || c == b'-' {
                    name.push(c as char);
                    self.pos += 1;
                } else {
                    break;
                }
            }
            segs.push(Seg::Tilde(name));
            at_word_start = false;
        }

        while let Some(b) = self.peek() {
            // Stop characters that end the word.
            if matches!(
                b,
                b' ' | b'\t' | b'\n' | b';' | b'&' | b'|' | b'<' | b'>' | b'(' | b')'
            ) {
                break;
            }

            // `#` only starts a comment at word start (i.e. after blanks).
            if b == b'#' && at_word_start && segs.is_empty() && lit.is_empty() {
                // Should have been skipped by `skip_blanks_and_comments`
                // already, but defensive: stop the word here.
                break;
            }

            match b {
                b'\\' => {
                    self.pos += 1;
                    if let Some(c) = self.peek() {
                        if c == b'\n' {
                            // Line continuation.
                            self.pos += 1;
                        } else {
                            lit.push(c as char);
                            self.pos += 1;
                        }
                    } else {
                        return Err(LexError::incomplete("unterminated backslash"));
                    }
                }
                b'\'' => {
                    flush(&mut segs, &mut lit);
                    self.pos += 1;
                    let body = self.read_single_quoted()?;
                    segs.push(Seg::SQuoted(body));
                }
                b'"' => {
                    flush(&mut segs, &mut lit);
                    self.pos += 1;
                    let inner = self.read_double_quoted()?;
                    segs.push(Seg::DQuoted(inner));
                }
                b'`' => {
                    flush(&mut segs, &mut lit);
                    self.pos += 1;
                    let body = self.read_backtick()?;
                    segs.push(Seg::CmdSub(body));
                }
                b'$' => {
                    flush(&mut segs, &mut lit);
                    let seg = self.read_dollar()?;
                    segs.push(seg);
                }
                _ => {
                    lit.push(b as char);
                    self.pos += 1;
                }
            }
            at_word_start = false;
        }
        flush(&mut segs, &mut lit);
        Ok(Word(segs))
    }

    fn read_single_quoted(&mut self) -> Result<String, LexError> {
        let mut out = String::new();
        while let Some(c) = self.peek() {
            if c == b'\'' {
                self.pos += 1;
                return Ok(out);
            }
            out.push(c as char);
            self.pos += 1;
        }
        Err(LexError::incomplete("unterminated single quote"))
    }

    fn read_double_quoted(&mut self) -> Result<Vec<Seg>, LexError> {
        let mut segs: Vec<Seg> = Vec::new();
        let mut lit = String::new();
        while let Some(c) = self.peek() {
            match c {
                b'"' => {
                    self.pos += 1;
                    flush(&mut segs, &mut lit);
                    return Ok(segs);
                }
                b'\\' => {
                    self.pos += 1;
                    if let Some(n) = self.peek() {
                        match n {
                            b'\n' => {
                                self.pos += 1;
                            }
                            b'$' | b'`' | b'"' | b'\\' => {
                                lit.push(n as char);
                                self.pos += 1;
                            }
                            _ => {
                                // Backslash before other characters in `"..."`
                                // is kept literally per POSIX.
                                lit.push('\\');
                                lit.push(n as char);
                                self.pos += 1;
                            }
                        }
                    } else {
                        return Err(LexError::incomplete("unterminated quoted backslash"));
                    }
                }
                b'`' => {
                    self.pos += 1;
                    flush(&mut segs, &mut lit);
                    let body = self.read_backtick()?;
                    segs.push(Seg::CmdSub(body));
                }
                b'$' => {
                    flush(&mut segs, &mut lit);
                    let seg = self.read_dollar()?;
                    segs.push(seg);
                }
                _ => {
                    lit.push(c as char);
                    self.pos += 1;
                }
            }
        }
        Err(LexError::incomplete("unterminated double quote"))
    }

    /// Reads everything inside a backtick command substitution, honoring the
    /// POSIX rule that `\` escapes `` ` ``, `$`, and `\`.
    fn read_backtick(&mut self) -> Result<String, LexError> {
        let mut out = String::new();
        while let Some(c) = self.peek() {
            if c == b'`' {
                self.pos += 1;
                return Ok(out);
            }
            if c == b'\\' {
                self.pos += 1;
                if let Some(n) = self.peek() {
                    if matches!(n, b'`' | b'$' | b'\\') {
                        out.push(n as char);
                        self.pos += 1;
                    } else {
                        out.push('\\');
                        out.push(n as char);
                        self.pos += 1;
                    }
                    continue;
                } else {
                    return Err(LexError::incomplete("unterminated backtick"));
                }
            }
            out.push(c as char);
            self.pos += 1;
        }
        Err(LexError::incomplete("unterminated backtick"))
    }

    /// Reads one `$`-prefixed expansion: simple variable, `${...}`,
    /// `$(...)`, or `$((...))`. The caller has already left the `$` at
    /// `self.pos`.
    fn read_dollar(&mut self) -> Result<Seg, LexError> {
        // consume the `$`
        self.pos += 1;
        match self.peek() {
            Some(b'(') => {
                // either `$((` or `$(`
                self.pos += 1;
                if self.peek() == Some(b'(') {
                    self.pos += 1;
                    let body = self.read_balanced("((", "))")?;
                    Ok(Seg::Arith(body))
                } else {
                    let body = self.read_balanced("(", ")")?;
                    Ok(Seg::CmdSub(body))
                }
            }
            Some(b'{') => {
                self.pos += 1;
                self.read_brace_param()
            }
            Some(c)
                if c == b'?'
                    || c == b'$'
                    || c == b'!'
                    || c == b'#'
                    || c == b'@'
                    || c == b'*'
                    || c.is_ascii_digit() =>
            {
                let mut name = String::new();
                name.push(c as char);
                self.pos += 1;
                Ok(Seg::Param {
                    name,
                    op: None,
                    arg: None,
                    length: false,
                })
            }
            Some(c) if is_name_start(c) => {
                let mut name = String::new();
                name.push(c as char);
                self.pos += 1;
                while let Some(n) = self.peek() {
                    if is_name_cont(n) {
                        name.push(n as char);
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                Ok(Seg::Param {
                    name,
                    op: None,
                    arg: None,
                    length: false,
                })
            }
            _ => {
                // Lone `$` — treat as literal.
                Ok(Seg::Lit("$".to_string()))
            }
        }
    }

    fn read_brace_param(&mut self) -> Result<Seg, LexError> {
        // `${#name}` length form
        let length = if self.peek() == Some(b'#') {
            // disambiguate from `${#}` which is the length-of-`$#` (parameter
            // count). If `#` is followed by `}`, treat `#` as the name.
            let save = self.pos;
            self.pos += 1;
            if self.peek() == Some(b'}') {
                self.pos = save;
                false
            } else {
                true
            }
        } else {
            false
        };

        // read the name
        let mut name = String::new();
        match self.peek() {
            Some(c)
                if c == b'?'
                    || c == b'$'
                    || c == b'!'
                    || c == b'#'
                    || c == b'@'
                    || c == b'*'
                    || c.is_ascii_digit() =>
            {
                name.push(c as char);
                self.pos += 1;
            }
            Some(c) if is_name_start(c) => {
                while let Some(n) = self.peek() {
                    if is_name_start(n) || n.is_ascii_digit() {
                        name.push(n as char);
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
            }
            _ => return Err(LexError::new("bad substitution")),
        }

        let op_byte = self.peek();
        // Optional operator + word
        let (op, arg) = match op_byte {
            Some(b'}') => (None, None),
            Some(b':') => {
                self.pos += 1;
                let next = self
                    .peek()
                    .ok_or_else(|| LexError::new("bad substitution"))?;
                let op = match next {
                    b'-' => ParamOp::DashNull,
                    b'=' => ParamOp::EqNull,
                    b'+' => ParamOp::PlusNull,
                    b'?' => ParamOp::QmarkNull,
                    _ => return Err(LexError::new("bad substitution")),
                };
                self.pos += 1;
                let arg = self.read_until_brace()?;
                (Some(op), Some(Box::new(arg)))
            }
            Some(b'-') => {
                self.pos += 1;
                let arg = self.read_until_brace()?;
                (Some(ParamOp::DashUnset), Some(Box::new(arg)))
            }
            Some(b'=') => {
                self.pos += 1;
                let arg = self.read_until_brace()?;
                (Some(ParamOp::EqUnset), Some(Box::new(arg)))
            }
            Some(b'+') => {
                self.pos += 1;
                let arg = self.read_until_brace()?;
                (Some(ParamOp::PlusUnset), Some(Box::new(arg)))
            }
            Some(b'?') => {
                self.pos += 1;
                let arg = self.read_until_brace()?;
                (Some(ParamOp::QmarkUnset), Some(Box::new(arg)))
            }
            _ => return Err(LexError::new("bad substitution")),
        };
        if self.peek() != Some(b'}') {
            return Err(LexError::incomplete("unterminated ${...}"));
        }
        self.pos += 1;
        Ok(Seg::Param {
            name,
            op,
            arg,
            length,
        })
    }

    /// Reads a word from inside `${...}` until the closing `}`, honoring
    /// nested quoting.
    fn read_until_brace(&mut self) -> Result<Word, LexError> {
        let mut segs: Vec<Seg> = Vec::new();
        let mut lit = String::new();
        let mut depth: i32 = 1;
        while let Some(c) = self.peek() {
            match c {
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        flush(&mut segs, &mut lit);
                        return Ok(Word(segs));
                    }
                    lit.push('}');
                    self.pos += 1;
                }
                b'{' => {
                    depth += 1;
                    lit.push('{');
                    self.pos += 1;
                }
                b'\\' => {
                    self.pos += 1;
                    if let Some(n) = self.peek() {
                        lit.push(n as char);
                        self.pos += 1;
                    }
                }
                b'\'' => {
                    flush(&mut segs, &mut lit);
                    self.pos += 1;
                    let body = self.read_single_quoted()?;
                    segs.push(Seg::SQuoted(body));
                }
                b'"' => {
                    flush(&mut segs, &mut lit);
                    self.pos += 1;
                    let inner = self.read_double_quoted()?;
                    segs.push(Seg::DQuoted(inner));
                }
                b'$' => {
                    flush(&mut segs, &mut lit);
                    let seg = self.read_dollar()?;
                    segs.push(seg);
                }
                b'`' => {
                    flush(&mut segs, &mut lit);
                    self.pos += 1;
                    let body = self.read_backtick()?;
                    segs.push(Seg::CmdSub(body));
                }
                _ => {
                    lit.push(c as char);
                    self.pos += 1;
                }
            }
        }
        Err(LexError::incomplete("unterminated ${...}"))
    }

    /// Reads text between balanced `open` and `close` delimiters, honoring
    /// quotes so a `)` inside `"..."` does not close `$(...)`.
    fn read_balanced(&mut self, open: &str, close: &str) -> Result<String, LexError> {
        let mut depth: i32 = 1;
        let mut out = String::new();
        let open_first = open.as_bytes()[0];
        let close_first = close.as_bytes()[0];
        let need_double = open.len() == 2 && close.len() == 2;
        while let Some(c) = self.peek() {
            // Honor quoted regions verbatim.
            if c == b'\'' {
                out.push('\'');
                self.pos += 1;
                while let Some(n) = self.peek() {
                    out.push(n as char);
                    self.pos += 1;
                    if n == b'\'' {
                        break;
                    }
                }
                continue;
            }
            if c == b'"' {
                out.push('"');
                self.pos += 1;
                while let Some(n) = self.peek() {
                    if n == b'\\' {
                        out.push('\\');
                        self.pos += 1;
                        if let Some(m) = self.peek() {
                            out.push(m as char);
                            self.pos += 1;
                        }
                        continue;
                    }
                    out.push(n as char);
                    self.pos += 1;
                    if n == b'"' {
                        break;
                    }
                }
                continue;
            }
            if c == b'\\' {
                out.push('\\');
                self.pos += 1;
                if let Some(n) = self.peek() {
                    out.push(n as char);
                    self.pos += 1;
                }
                continue;
            }
            if need_double {
                if c == open_first && self.peek_at(1) == Some(open.as_bytes()[1]) {
                    depth += 1;
                    out.push_str(open);
                    self.pos += 2;
                    continue;
                }
                if c == close_first && self.peek_at(1) == Some(close.as_bytes()[1]) {
                    depth -= 1;
                    if depth == 0 {
                        self.pos += 2;
                        return Ok(out);
                    }
                    out.push_str(close);
                    self.pos += 2;
                    continue;
                }
            } else {
                if c == open_first {
                    depth += 1;
                    out.push(c as char);
                    self.pos += 1;
                    continue;
                }
                if c == close_first {
                    depth -= 1;
                    if depth == 0 {
                        self.pos += 1;
                        return Ok(out);
                    }
                    out.push(c as char);
                    self.pos += 1;
                    continue;
                }
            }
            out.push(c as char);
            self.pos += 1;
        }
        Err(LexError::incomplete("unterminated ${} / $() / $(()"))
    }

    fn skip_blanks_and_comments(&mut self) {
        loop {
            match self.peek() {
                Some(b' ') | Some(b'\t') => self.pos += 1,
                Some(b'\\') if self.peek_at(1) == Some(b'\n') => self.pos += 2,
                Some(b'#') => {
                    while let Some(c) = self.peek() {
                        if c == b'\n' {
                            break;
                        }
                        self.pos += 1;
                    }
                }
                _ => break,
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_at(&self, off: usize) -> Option<u8> {
        self.bytes.get(self.pos + off).copied()
    }
}

fn flush(segs: &mut Vec<Seg>, lit: &mut String) {
    if !lit.is_empty() {
        segs.push(Seg::Lit(core::mem::take(lit)));
    }
}

fn is_name_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_name_cont(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}
