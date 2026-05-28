//! Declarative lexer toolkit.
//!
//! You build a [`Lexer`] by chaining `.skip(...)` rules (whitespace,
//! comments, anything else to discard) and `.rule(...)` rules (anything
//! that produces a token). Each rule is a closure that inspects a
//! [`Cursor`] and reports either `None` (this rule doesn't apply here)
//! or `Some((bytes_consumed, token_kind))`. `Lexer::tokenize` drives the
//! whole thing, threading source-position bookkeeping for you so error
//! messages always carry a line / column.
//!
//! Most of the time you don't write closures directly — the helpers
//! [`keyword`], [`ident`], [`int`], [`quoted_string`], [`line_comment`]
//! and friends cover the common cases.
//!
//! ## Sketch of usage
//!
//! ```no_run
//! use user_program::compiler::lex::*;
//!
//! #[derive(Clone, Copy, Debug, PartialEq, Eq)]
//! enum Tok {
//!     Ident,
//!     Int,
//!     Plus,
//!     Minus,
//!     KwIf,
//! }
//! impl TokenKind for Tok {
//!     fn name(self) -> &'static str {
//!         match self {
//!             Tok::Ident => "identifier",
//!             Tok::Int => "integer",
//!             Tok::Plus => "`+`",
//!             Tok::Minus => "`-`",
//!             Tok::KwIf => "`if`",
//!         }
//!     }
//! }
//!
//! let lexer = Lexer::<Tok>::new()
//!     .skip(whitespace())
//!     .skip(line_comment("//"))
//!     .rule(keyword("if", Tok::KwIf))
//!     .rule(symbol("+", Tok::Plus))
//!     .rule(symbol("-", Tok::Minus))
//!     .rule(ident(Tok::Ident))
//!     .rule(int(Tok::Int));
//!
//! let tokens = lexer.tokenize("if x + 1").unwrap();
//! ```

use alloc::{
    boxed::Box,
    string::{String, ToString},
    vec::Vec,
};
use core::fmt;

/// Trait every token-kind enum has to implement so error messages can
/// render it with a friendly name. Keep the strings short and quoted
/// where appropriate (e.g. `"`if`"`, `"identifier"`).
pub trait TokenKind: Copy + 'static {
    fn name(self) -> &'static str;
}

/// One token produced by the lexer.
#[derive(Clone, Debug)]
pub struct Token<Tok> {
    pub kind: Tok,
    /// The matched source slice, owned for ergonomics. Copying a small
    /// string per token is fine for the file sizes we work on.
    pub text: String,
    /// 1-based line and column of the token's first character.
    pub line: u32,
    pub col: u32,
}

/// A read-only view into the remaining source plus the current
/// line / column counter. Returned to every lexer rule.
#[derive(Clone, Copy)]
pub struct Cursor<'a> {
    pub rest: &'a str,
    pub line: u32,
    pub col: u32,
}

impl Cursor<'_> {
    /// Returns the first byte without consuming it.
    pub fn peek(&self) -> Option<u8> {
        self.rest.as_bytes().first().copied()
    }

    /// Returns `true` if the remaining text starts with `prefix`.
    pub fn starts_with(&self, prefix: &str) -> bool {
        self.rest.starts_with(prefix)
    }

    /// True at end of input.
    pub fn at_eof(&self) -> bool {
        self.rest.is_empty()
    }

    /// Counts how many leading bytes satisfy `pred`. Does **not**
    /// advance the cursor.
    pub fn count_while<F: FnMut(u8) -> bool>(&self, mut pred: F) -> usize {
        self.rest
            .as_bytes()
            .iter()
            .take_while(|b| pred(**b))
            .count()
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Lex-time failure. Carries the offending position so the caller can
/// report `line:col: <message>` without extra bookkeeping.
#[derive(Clone, Debug)]
pub struct LexError {
    pub line: u32,
    pub col: u32,
    pub message: String,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.col, self.message)
    }
}

// ---------------------------------------------------------------------------
// Rule and Lexer
// ---------------------------------------------------------------------------

/// The two outcomes a rule can produce.
pub enum Match<Tok> {
    /// Consumed `bytes`, emit a token of this kind. The lexer carves
    /// `bytes` out of the source as the token's text.
    Emit { bytes: usize, kind: Tok },
    /// Consumed `bytes` of "skippable" input (whitespace, comment).
    Skip { bytes: usize },
}

/// Boxed rule closure. Public so user-written matchers fit the type.
pub type Rule<Tok> = Box<dyn Fn(&Cursor<'_>) -> Option<Match<Tok>>>;

/// Declarative lexer. Built up rule by rule.
///
/// Rules are tried in registration order on every position; the first
/// one that matches wins. Put more specific rules (e.g. `==` before
/// `=`, keywords before identifiers) earlier in the chain.
pub struct Lexer<Tok: TokenKind> {
    rules: Vec<Rule<Tok>>,
}

impl<Tok: TokenKind> Default for Lexer<Tok> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Tok: TokenKind> Lexer<Tok> {
    /// Creates an empty lexer. Use [`skip`](Self::skip) and
    /// [`rule`](Self::rule) to populate it.
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    /// Adds a rule that consumes input without producing a token —
    /// whitespace, comments, etc.
    pub fn skip<F>(mut self, matcher: F) -> Self
    where F: Fn(&Cursor<'_>) -> Option<usize> + 'static {
        self.rules.push(Box::new(move |cur| {
            matcher(cur).map(|bytes| Match::Skip { bytes })
        }));
        self
    }

    /// Adds a rule that, when matched, emits a token.
    pub fn rule<F>(mut self, matcher: F) -> Self
    where F: Fn(&Cursor<'_>) -> Option<(usize, Tok)> + 'static {
        self.rules.push(Box::new(move |cur| {
            matcher(cur).map(|(bytes, kind)| Match::Emit { bytes, kind })
        }));
        self
    }

    /// Convenience: append a batch of `keyword` rules in one call.
    pub fn keywords(mut self, table: &[(&'static str, Tok)]) -> Self {
        for &(text, kind) in table {
            let rule = keyword(text, kind);
            self.rules.push(Box::new(move |cur| {
                rule(cur).map(|(bytes, kind)| Match::Emit { bytes, kind })
            }));
        }
        self
    }

    /// Same as [`Self::keywords`] but for plain symbols (no
    /// identifier-tail check).
    pub fn symbols(mut self, table: &[(&'static str, Tok)]) -> Self {
        for &(text, kind) in table {
            let rule = symbol(text, kind);
            self.rules.push(Box::new(move |cur| {
                rule(cur).map(|(bytes, kind)| Match::Emit { bytes, kind })
            }));
        }
        self
    }

    /// Drives the lexer over `source` and returns every emitted token,
    /// or the first [`LexError`] we hit.
    pub fn tokenize(&self, source: &str) -> Result<Vec<Token<Tok>>, LexError> {
        let mut cur = Cursor {
            rest: source,
            line: 1,
            col: 1,
        };
        let mut out = Vec::new();
        while !cur.at_eof() {
            let line = cur.line;
            let col = cur.col;
            let mut matched = false;
            for rule in &self.rules {
                if let Some(m) = rule(&cur) {
                    matched = true;
                    match m {
                        Match::Emit { bytes, kind } => {
                            let text = cur.rest[..bytes].to_string();
                            out.push(Token {
                                kind,
                                text,
                                line,
                                col,
                            });
                            advance(&mut cur, bytes);
                        }
                        Match::Skip { bytes } => advance(&mut cur, bytes),
                    }
                    break;
                }
            }
            if !matched {
                let ch = cur.rest.chars().next().unwrap_or('\0');
                return Err(LexError {
                    line,
                    col,
                    message: alloc::format!("unexpected character `{}`", ch),
                });
            }
        }
        Ok(out)
    }
}

/// Moves the cursor past `bytes`, keeping `line` / `col` in sync.
fn advance(cur: &mut Cursor<'_>, bytes: usize) {
    let consumed = &cur.rest[..bytes];
    for b in consumed.bytes() {
        if b == b'\n' {
            cur.line += 1;
            cur.col = 1;
        } else {
            cur.col += 1;
        }
    }
    cur.rest = &cur.rest[bytes..];
}

// ---------------------------------------------------------------------------
// Built-in rule helpers
// ---------------------------------------------------------------------------

/// Skip ASCII whitespace (spaces, tabs, CR, LF). Use with
/// [`Lexer::skip`].
pub fn whitespace() -> impl Fn(&Cursor<'_>) -> Option<usize> {
    |cur| {
        let n = cur.count_while(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'));
        if n == 0 { None } else { Some(n) }
    }
}

/// Skip a line comment introduced by `marker` (e.g. `"//"`, `"#"`,
/// `"--"`). Consumes through the terminating `\n` (or EOF).
pub fn line_comment(marker: &'static str) -> impl Fn(&Cursor<'_>) -> Option<usize> {
    move |cur| {
        if !cur.starts_with(marker) {
            return None;
        }
        let bytes = cur.rest.as_bytes();
        let mut i = marker.len();
        while i < bytes.len() && bytes[i] != b'\n' {
            i += 1;
        }
        // Consume the newline too if present.
        if i < bytes.len() {
            i += 1;
        }
        Some(i)
    }
}

/// Skip a block comment delimited by `open` and `close` (e.g.
/// `block_comment("/*", "*/")`). Reports a [`LexError`] indirectly by
/// just not matching — the lexer's "no rule matched" path will then
/// blame the opener position, which is what users want.
pub fn block_comment(
    open: &'static str,
    close: &'static str,
) -> impl Fn(&Cursor<'_>) -> Option<usize> {
    move |cur| {
        if !cur.starts_with(open) {
            return None;
        }
        let bytes = cur.rest.as_bytes();
        let close_bytes = close.as_bytes();
        let mut i = open.len();
        while i + close_bytes.len() <= bytes.len() {
            if &bytes[i..i + close_bytes.len()] == close_bytes {
                return Some(i + close_bytes.len());
            }
            i += 1;
        }
        // Unterminated. Consume to EOF so the caller doesn't loop.
        Some(bytes.len())
    }
}

/// Match an exact literal keyword and require that the character after
/// it is **not** part of an identifier. Use for reserved words like
/// `"if"`, `"while"`.
pub fn keyword<Tok: TokenKind>(
    word: &'static str,
    kind: Tok,
) -> impl Fn(&Cursor<'_>) -> Option<(usize, Tok)> {
    move |cur| {
        if !cur.starts_with(word) {
            return None;
        }
        let after = cur.rest.as_bytes().get(word.len()).copied();
        if matches!(after, Some(b) if is_ident_cont(b)) {
            return None;
        }
        Some((word.len(), kind))
    }
}

/// Match an exact literal symbol with no trailing-character constraint.
/// Use for punctuation like `";"`, `"=="`.
pub fn symbol<Tok: TokenKind>(
    text: &'static str,
    kind: Tok,
) -> impl Fn(&Cursor<'_>) -> Option<(usize, Tok)> {
    move |cur| {
        if cur.starts_with(text) {
            Some((text.len(), kind))
        } else {
            None
        }
    }
}

/// Match a C-style identifier: `[A-Za-z_][A-Za-z0-9_]*`. The whole
/// identifier emits one token of `kind`.
pub fn ident<Tok: TokenKind>(kind: Tok) -> impl Fn(&Cursor<'_>) -> Option<(usize, Tok)> {
    move |cur| {
        let bytes = cur.rest.as_bytes();
        let first = *bytes.first()?;
        if !is_ident_start(first) {
            return None;
        }
        let len = 1 + cur.count_while_skipping(1, is_ident_cont);
        Some((len, kind))
    }
}

/// Match a decimal / hex / octal integer literal.
///
/// - `0x` / `0X` prefix → hex digits (`0-9A-Fa-f`)
/// - leading `0` followed by octal digits → octal
/// - otherwise → decimal
pub fn int<Tok: TokenKind>(kind: Tok) -> impl Fn(&Cursor<'_>) -> Option<(usize, Tok)> {
    move |cur| {
        let bytes = cur.rest.as_bytes();
        if !matches!(bytes.first(), Some(b) if b.is_ascii_digit()) {
            return None;
        }
        let len = if bytes.len() >= 2 && bytes[0] == b'0' && (bytes[1] == b'x' || bytes[1] == b'X')
        {
            2 + cur.count_while_skipping(2, |b| b.is_ascii_hexdigit())
        } else if bytes[0] == b'0' {
            1 + cur.count_while_skipping(1, |b| (b'0'..=b'7').contains(&b))
        } else {
            cur.count_while(|b| b.is_ascii_digit())
        };
        Some((len, kind))
    }
}

/// Match a string literal delimited by `quote`. Supports backslash
/// escapes via `escape`: `"\n"`, `"\\"`, `"\""`, etc. The token's
/// `text` field includes the surrounding quotes, leaving unescaping
/// for a later AST pass.
pub fn quoted_string<Tok: TokenKind>(
    quote: char,
    escape: char,
    kind: Tok,
) -> impl Fn(&Cursor<'_>) -> Option<(usize, Tok)> {
    move |cur| {
        let bytes = cur.rest.as_bytes();
        if bytes.first().copied() != Some(quote as u8) {
            return None;
        }
        let mut i = 1;
        while i < bytes.len() {
            if bytes[i] == escape as u8 && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            if bytes[i] == quote as u8 {
                return Some((i + 1, kind));
            }
            i += 1;
        }
        // Unterminated; let the caller see something rather than loop
        // forever. We consume what's left.
        Some((bytes.len(), kind))
    }
}

impl Cursor<'_> {
    /// Like [`Self::count_while`] but starts counting after `skip`
    /// initial bytes. Useful for `ident` / `int` which check the first
    /// character separately.
    fn count_while_skipping<F: FnMut(u8) -> bool>(&self, skip: usize, mut pred: F) -> usize {
        self.rest
            .as_bytes()
            .iter()
            .skip(skip)
            .take_while(|b| pred(**b))
            .count()
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
