//! Declarative parser combinators on top of [`crate::lex::Token`].
//!
//! A [`Parser<Tok, T>`] is a boxed function that walks a token stream
//! and returns either `Ok(value)` or a [`ParseError`] with line / column
//! information. You compose them with [`seq`], [`choice`], [`optional`],
//! [`many`], [`sep_by`], and friends; you describe AST shapes with
//! [`Parser::map`]. Operator-precedence is handled separately with
//! [`pratt`].
//!
//! Every combinator that "tries an alternative" rewinds the stream on
//! failure, so the user never has to think about backtracking. The
//! error reported by [`choice`] is the **deepest** one of its
//! alternatives — the one that consumed the most tokens before failing —
//! matching what most modern combinator libraries do and what real users
//! find useful.
//!
//! ## A taste
//!
//! ```ignore
//! use user_program::compiler::parse::*;
//! use user_program::compiler::lex::*;
//!
//! enum Stmt { If(Box<Expr>, Vec<Stmt>), /* … */ }
//!
//! fn stmt() -> Parser<Tok, Stmt> {
//!     choice((
//!         seq((tok(KwIf), expr(), tok(KwThen), many(stmt()), tok(KwEnd)))
//!             .map(|(_, c, _, body, _)| Stmt::If(Box::new(c), body)),
//!         // … other statement forms …
//!     ))
//!     .labeled("statement")
//! }
//! ```

use alloc::{
    rc::Rc,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use core::fmt;

use crate::compiler::lex::{Token, TokenKind};

// ---------------------------------------------------------------------------
// Stream + ParseError
// ---------------------------------------------------------------------------

/// Position-aware token stream. Internal — parsers receive `&mut Stream`.
pub struct Stream<'a, Tok: TokenKind> {
    tokens: &'a [Token<Tok>],
    pos: usize,
    /// "Deepest failure so far": the furthest position any failing
    /// alternative reached. Used to give `choice` good error messages.
    farthest: usize,
    farthest_err: Option<ParseError>,
}

impl<'a, Tok: TokenKind> Stream<'a, Tok> {
    pub fn new(tokens: &'a [Token<Tok>]) -> Self {
        Self {
            tokens,
            pos: 0,
            farthest: 0,
            farthest_err: None,
        }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn rewind(&mut self, pos: usize) {
        self.pos = pos;
    }

    pub fn peek(&self) -> Option<&Token<Tok>> {
        self.tokens.get(self.pos)
    }

    pub fn bump(&mut self) -> Option<&Token<Tok>> {
        let t = self.tokens.get(self.pos)?;
        self.pos += 1;
        Some(t)
    }

    /// Records `err` as a potential "deepest failure" if it actually is
    /// the deepest one we've seen.
    pub fn record(&mut self, err: ParseError) {
        if self.pos >= self.farthest {
            self.farthest = self.pos;
            self.farthest_err = Some(err);
        }
    }

    /// Builds a fresh error pointing at the current token (or EOF).
    pub fn error(&self, expected: &str) -> ParseError {
        match self.tokens.get(self.pos) {
            Some(t) => ParseError {
                line: t.line,
                col: t.col,
                expected: expected.to_string(),
                found: alloc::format!("`{}`", t.text),
            },
            None => {
                let (line, col) = self
                    .tokens
                    .last()
                    .map(|t| (t.line, t.col + t.text.len() as u32))
                    .unwrap_or((1, 1));
                ParseError {
                    line,
                    col,
                    expected: expected.to_string(),
                    found: "end of input".to_string(),
                }
            }
        }
    }
}

/// One parse failure. Always carries a position and an
/// `expected X, found Y` pair so the user can spot the issue.
#[derive(Clone, Debug)]
pub struct ParseError {
    pub line: u32,
    pub col: u32,
    pub expected: String,
    pub found: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "parse error at line {}, col {}: expected {}, found {}",
            self.line, self.col, self.expected, self.found
        )
    }
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Type alias used internally — the boxed closure that powers
/// [`Parser`]. Keeps clippy's `type_complexity` lint quiet and matches
/// what every other combinator library calls this thing.
type RunFn<Tok, T> = dyn Fn(&mut Stream<'_, Tok>) -> Result<T, ParseError>;

/// Boxed parser. Shared by `Rc` so combinators can clone cheaply.
pub struct Parser<Tok: TokenKind, T> {
    run: Rc<RunFn<Tok, T>>,
}

impl<Tok: TokenKind, T> Clone for Parser<Tok, T> {
    fn clone(&self) -> Self {
        Self {
            run: Rc::clone(&self.run),
        }
    }
}

impl<Tok: TokenKind, T: 'static> Parser<Tok, T> {
    /// Builds a parser from a closure. Mostly for advanced use — the
    /// helpers below cover normal cases.
    pub fn new<F>(f: F) -> Self
    where F: Fn(&mut Stream<'_, Tok>) -> Result<T, ParseError> + 'static {
        Self { run: Rc::new(f) }
    }

    /// Runs the parser. Returns the value and the next unconsumed
    /// token's index, or the deepest error we observed.
    pub fn parse(&self, tokens: &[Token<Tok>]) -> Result<T, ParseError> {
        let mut stream = Stream::new(tokens);
        let r = (self.run)(&mut stream);
        match r {
            Ok(v) if stream.pos == tokens.len() => Ok(v),
            Ok(_) => Err(stream.error("end of input")),
            Err(err) => Err(stream.farthest_err.unwrap_or(err)),
        }
    }

    /// Transforms the result. The classic `fmap`.
    pub fn map<U: 'static, F>(self, f: F) -> Parser<Tok, U>
    where F: Fn(T) -> U + 'static {
        Parser::new(move |s| (self.run)(s).map(&f))
    }

    /// Overrides the "expected" text used in errors at this point.
    pub fn labeled(self, what: &'static str) -> Parser<Tok, T> {
        Parser::new(move |s| {
            let start = s.pos();
            (self.run)(s).map_err(|_| {
                s.rewind(start);
                let err = s.error(what);
                s.record(err.clone());
                err
            })
        })
    }
}

// ---------------------------------------------------------------------------
// Primitive parsers
// ---------------------------------------------------------------------------

/// Matches exactly one token of `kind`. The matched token is returned.
pub fn tok<Tok: TokenKind + PartialEq>(kind: Tok) -> Parser<Tok, Token<Tok>> {
    Parser::new(move |s| {
        let start = s.pos();
        match s.peek() {
            Some(t) if t.kind == kind => {
                let t = t.clone();
                s.bump();
                Ok(t)
            }
            _ => {
                s.rewind(start);
                let err = s.error(kind.name());
                s.record(err.clone());
                Err(err)
            }
        }
    })
}

/// Matches the next token if `pred` accepts it. Useful for "any of N
/// kinds" without writing `choice`.
pub fn tok_if<Tok, F>(pred: F, label: &'static str) -> Parser<Tok, Token<Tok>>
where
    Tok: TokenKind,
    F: Fn(Tok) -> bool + 'static,
{
    Parser::new(move |s| {
        let start = s.pos();
        match s.peek() {
            Some(t) if pred(t.kind) => {
                let t = t.clone();
                s.bump();
                Ok(t)
            }
            _ => {
                s.rewind(start);
                let err = s.error(label);
                s.record(err.clone());
                Err(err)
            }
        }
    })
}

/// Always succeeds with `value`, consuming no input. Useful for
/// "default value" in `optional`-like patterns.
pub fn pure<Tok: TokenKind, T: Clone + 'static>(value: T) -> Parser<Tok, T> {
    Parser::new(move |_| Ok(value.clone()))
}

// ---------------------------------------------------------------------------
// Sequence — implemented via a small trait so users can write tuples
// of arbitrary arity without writing seq2/seq3/seq4 by hand.
// ---------------------------------------------------------------------------

/// Sequences a tuple of parsers and returns a tuple of their outputs.
/// Supports tuples of arity 2 through 8.
pub fn seq<Tok: TokenKind, S: Sequence<Tok>>(parsers: S) -> Parser<Tok, S::Output> {
    S::into_parser(parsers)
}

/// Marker trait that powers [`seq`]. Implemented for tuples of
/// `Parser`s; the implementation runs each parser in turn and assembles
/// a result tuple, rolling back on the first failure.
pub trait Sequence<Tok: TokenKind>: Sized + 'static {
    type Output: 'static;
    fn into_parser(self) -> Parser<Tok, Self::Output>;
}

macro_rules! impl_sequence {
    ($($name:ident),+) => {
        impl<Tok, $($name),+> Sequence<Tok> for ($(Parser<Tok, $name>,)+)
        where
            Tok: TokenKind + 'static,
            $($name: 'static,)+
        {
            type Output = ($($name,)+);
            #[allow(non_snake_case)]
            fn into_parser(self) -> Parser<Tok, Self::Output> {
                let ( $($name,)+ ) = self;
                Parser::new(move |s| {
                    let start = s.pos();
                    let result = (|| -> Result<Self::Output, ParseError> {
                        Ok(( $((($name.run)(s))?,)+ ))
                    })();
                    if result.is_err() {
                        s.rewind(start);
                    }
                    result
                })
            }
        }
    };
}

impl_sequence!(A, B);
impl_sequence!(A, B, C);
impl_sequence!(A, B, C, D);
impl_sequence!(A, B, C, D, E);
impl_sequence!(A, B, C, D, E, F);
impl_sequence!(A, B, C, D, E, F, G);
impl_sequence!(A, B, C, D, E, F, G, H);

// ---------------------------------------------------------------------------
// Choice
// ---------------------------------------------------------------------------

/// Tries each parser in turn; the first one to succeed wins. All
/// alternatives must produce the same `T`.
pub fn choice<Tok: TokenKind, S: ChoiceTuple<Tok>>(alternatives: S) -> Parser<Tok, S::Output> {
    S::into_parser(alternatives)
}

pub trait ChoiceTuple<Tok: TokenKind>: Sized + 'static {
    type Output: 'static;
    fn into_parser(self) -> Parser<Tok, Self::Output>;
}

/// Helper macro: `replace_ident!(foo, T)` → `T`. We use it inside other
/// macros to make `$name` appear (so rustc accepts the repetition) even
/// though we only care about the count.
macro_rules! replace_ident {
    ($_ignored:ident, $($body:tt)*) => { $($body)* };
}

macro_rules! impl_choice {
    ($($name:ident),+) => {
        impl<Tok, T> ChoiceTuple<Tok> for ($(replace_ident!($name, Parser<Tok, T>),)+)
        where
            Tok: TokenKind + 'static,
            T: 'static,
        {
            type Output = T;
            #[allow(non_snake_case, unused_assignments)]
            fn into_parser(self) -> Parser<Tok, Self::Output> {
                let ( $($name,)+ ) = self;
                Parser::new(move |s| {
                    let start = s.pos();
                    let mut last: Option<ParseError> = None;
                    $(
                        s.rewind(start);
                        match ($name.run)(s) {
                            Ok(v) => return Ok(v),
                            Err(e) => last = Some(e),
                        }
                    )+
                    s.rewind(start);
                    Err(last.unwrap())
                })
            }
        }
    };
}

impl_choice!(A);
impl_choice!(A, B);
impl_choice!(A, B, C);
impl_choice!(A, B, C, D);
impl_choice!(A, B, C, D, E);
impl_choice!(A, B, C, D, E, F);
impl_choice!(A, B, C, D, E, F, G);
impl_choice!(A, B, C, D, E, F, G, H);

// ---------------------------------------------------------------------------
// Quantifiers
// ---------------------------------------------------------------------------

/// `[X]` — zero or one occurrence; returns `Option<T>`.
pub fn optional<Tok: TokenKind + 'static, T: 'static>(p: Parser<Tok, T>) -> Parser<Tok, Option<T>> {
    Parser::new(move |s| {
        let start = s.pos();
        match (p.run)(s) {
            Ok(v) => Ok(Some(v)),
            Err(_) => {
                s.rewind(start);
                Ok(None)
            }
        }
    })
}

/// `{X}` — zero or more occurrences; returns `Vec<T>`.
pub fn many<Tok: TokenKind + 'static, T: 'static>(p: Parser<Tok, T>) -> Parser<Tok, Vec<T>> {
    Parser::new(move |s| {
        let mut out = Vec::new();
        loop {
            let start = s.pos();
            match (p.run)(s) {
                Ok(v) => {
                    if s.pos() == start {
                        // No progress — the parser matched without
                        // consuming. Stop to avoid an infinite loop.
                        out.push(v);
                        break;
                    }
                    out.push(v);
                }
                Err(_) => {
                    s.rewind(start);
                    break;
                }
            }
        }
        Ok(out)
    })
}

/// `X {X}` — one or more occurrences.
pub fn many1<Tok: TokenKind + 'static, T: 'static>(p: Parser<Tok, T>) -> Parser<Tok, Vec<T>> {
    Parser::new(move |s| {
        let first = (p.run)(s)?;
        let mut out = vec![first];
        loop {
            let start = s.pos();
            match (p.run)(s) {
                Ok(v) => {
                    if s.pos() == start {
                        out.push(v);
                        break;
                    }
                    out.push(v);
                }
                Err(_) => {
                    s.rewind(start);
                    break;
                }
            }
        }
        Ok(out)
    })
}

/// `X (sep X)*` — zero or more `p` separated by `sep`; `sep` results
/// are discarded. Returns an empty `Vec` for "no match".
pub fn sep_by<Tok, T, S>(p: Parser<Tok, T>, sep: Parser<Tok, S>) -> Parser<Tok, Vec<T>>
where
    Tok: TokenKind + 'static,
    T: 'static,
    S: 'static,
{
    Parser::new(move |stream| {
        let start = stream.pos();
        let first = match (p.run)(stream) {
            Ok(v) => v,
            Err(_) => {
                stream.rewind(start);
                return Ok(Vec::new());
            }
        };
        let mut out = vec![first];
        loop {
            let mark = stream.pos();
            if (sep.run)(stream).is_err() {
                stream.rewind(mark);
                return Ok(out);
            }
            match (p.run)(stream) {
                Ok(v) => out.push(v),
                Err(err) => {
                    // Saw a separator but no following element — that's
                    // a real syntax error, not a clean end.
                    return Err(err);
                }
            }
        }
    })
}

/// Like [`sep_by`] but requires at least one element.
pub fn sep_by1<Tok, T, S>(p: Parser<Tok, T>, sep: Parser<Tok, S>) -> Parser<Tok, Vec<T>>
where
    Tok: TokenKind + 'static,
    T: 'static,
    S: 'static,
{
    Parser::new(move |stream| {
        let first = (p.run)(stream)?;
        let mut out = vec![first];
        loop {
            let mark = stream.pos();
            if (sep.run)(stream).is_err() {
                stream.rewind(mark);
                return Ok(out);
            }
            out.push((p.run)(stream)?);
        }
    })
}

/// Delays construction of a parser until first use — break recursion
/// (e.g. `expr → expr '+' expr`) by writing `lazy(expr)`.
pub fn lazy<Tok, T, F>(make: F) -> Parser<Tok, T>
where
    Tok: TokenKind + 'static,
    T: 'static,
    F: Fn() -> Parser<Tok, T> + 'static,
{
    Parser::new(move |s| (make().run)(s))
}

// ---------------------------------------------------------------------------
// Pratt builder for operator-precedence expressions
// ---------------------------------------------------------------------------

/// Builds a Pratt-style expression parser from an atom parser plus a
/// table of prefix / infix / postfix operators. Each operator carries a
/// "binding power" — higher binds tighter — and a fold closure that
/// turns sub-results into the next AST level.
///
/// ```ignore
/// fn expr() -> Parser<Tok, Expr> {
///     pratt(atom())
///         .prefix(Tok::Minus, |e| Expr::Neg(Box::new(e)))
///         .infix_left(Tok::Plus,  10, |a, b| Expr::Add(Box::new(a), Box::new(b)))
///         .infix_left(Tok::Star,  20, |a, b| Expr::Mul(Box::new(a), Box::new(b)))
///         .build()
/// }
/// ```
pub fn pratt<Tok, T>(atom: Parser<Tok, T>) -> PrattBuilder<Tok, T>
where
    Tok: TokenKind + PartialEq + 'static,
    T: 'static,
{
    PrattBuilder {
        atom,
        prefix: Vec::new(),
        infix: Vec::new(),
        postfix: Vec::new(),
    }
}

type PrefixFn<T> = Rc<dyn Fn(T) -> T>;
type InfixFn<T> = Rc<dyn Fn(T, T) -> T>;
type PostfixFn<T> = Rc<dyn Fn(T) -> T>;

pub struct PrattBuilder<Tok: TokenKind + PartialEq, T> {
    atom: Parser<Tok, T>,
    prefix: Vec<(Tok, u32, PrefixFn<T>)>,
    /// `(token, lbp, rbp, fold)` — `lbp == rbp` for left-assoc,
    /// `rbp = lbp - 1` for right-assoc (handled by `infix_right`).
    infix: Vec<(Tok, u32, u32, InfixFn<T>)>,
    postfix: Vec<(Tok, u32, PostfixFn<T>)>,
}

impl<Tok, T> PrattBuilder<Tok, T>
where
    Tok: TokenKind + PartialEq + 'static,
    T: 'static,
{
    /// Adds a prefix operator. Conventionally use a binding power
    /// higher than every infix one so `-1 + 2` parses as `(-1) + 2`.
    pub fn prefix<F>(mut self, kind: Tok, fold: F) -> Self
    where F: Fn(T) -> T + 'static {
        // Default prefix bp: higher than typical infix range.
        self.prefix.push((kind, 100, Rc::new(fold)));
        self
    }

    /// Adds a left-associative infix operator at binding power `bp`.
    pub fn infix_left<F>(mut self, kind: Tok, bp: u32, fold: F) -> Self
    where F: Fn(T, T) -> T + 'static {
        self.infix.push((kind, bp, bp + 1, Rc::new(fold)));
        self
    }

    /// Adds a right-associative infix operator at binding power `bp`.
    pub fn infix_right<F>(mut self, kind: Tok, bp: u32, fold: F) -> Self
    where F: Fn(T, T) -> T + 'static {
        // For right-assoc, rbp == lbp, so a chain like `a = b = c`
        // groups as `a = (b = c)`.
        self.infix.push((kind, bp + 1, bp, Rc::new(fold)));
        self
    }

    /// Adds a postfix operator.
    pub fn postfix<F>(mut self, kind: Tok, fold: F) -> Self
    where F: Fn(T) -> T + 'static {
        self.postfix.push((kind, 100, Rc::new(fold)));
        self
    }

    pub fn build(self) -> Parser<Tok, T> {
        let atom = self.atom;
        let prefix = self.prefix;
        let infix = self.infix;
        let postfix = self.postfix;
        Parser::new(move |s| pratt_parse(s, &atom, &prefix, &infix, &postfix, 0))
    }
}

fn pratt_parse<Tok, T>(
    s: &mut Stream<'_, Tok>,
    atom: &Parser<Tok, T>,
    prefix: &[(Tok, u32, PrefixFn<T>)],
    infix: &[(Tok, u32, u32, InfixFn<T>)],
    postfix: &[(Tok, u32, PostfixFn<T>)],
    min_bp: u32,
) -> Result<T, ParseError>
where
    Tok: TokenKind + PartialEq + 'static,
    T: 'static,
{
    // Prefix or atom.
    let peek = s.peek().map(|t| t.kind);
    let mut lhs = if let Some(k) = peek {
        if let Some((_, bp, fold)) = prefix.iter().find(|(t, _, _)| *t == k) {
            s.bump();
            let rhs = pratt_parse(s, atom, prefix, infix, postfix, *bp)?;
            fold(rhs)
        } else {
            (atom.run)(s)?
        }
    } else {
        (atom.run)(s)?
    };

    // Loop on infix / postfix.
    loop {
        let Some(k) = s.peek().map(|t| t.kind) else {
            break;
        };

        if let Some((_, lbp, fold)) = postfix.iter().find(|(t, _, _)| *t == k) {
            if *lbp < min_bp {
                break;
            }
            s.bump();
            lhs = fold(lhs);
            continue;
        }

        if let Some((_, lbp, rbp, fold)) = infix.iter().find(|(t, _, _, _)| *t == k) {
            if *lbp < min_bp {
                break;
            }
            s.bump();
            let rhs = pratt_parse(s, atom, prefix, infix, postfix, *rbp)?;
            lhs = fold(lhs, rhs);
            continue;
        }
        break;
    }
    Ok(lhs)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
