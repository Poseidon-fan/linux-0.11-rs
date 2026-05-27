//! Parser.
//!
//! Recursive-descent parser turning a token stream from [`crate::lexer`]
//! into a [`Cmd`](crate::ast::Cmd) AST. The grammar implemented is a
//! POSIX-shell subset, omitting `case` / `until` / here-documents.

use alloc::{
    boxed::Box,
    string::{String, ToString},
    vec::Vec,
};

use crate::{
    ast::{AndOrOp, Assignment, Cmd, Redir, RedirOp, RedirTarget, Seg, Sep, SimpleCmd, Word},
    lexer::{LexError, Lexer, Token},
};

/// Drives the recursive descent over a lexer.
pub struct Parser<'a> {
    lex: Lexer<'a>,
    /// One-token lookahead.
    cur: Token,
}

impl<'a> Parser<'a> {
    /// Build a parser from source text.
    pub fn new(src: &'a str) -> Result<Self, LexError> {
        let mut lex = Lexer::new(src);
        let cur = lex.next_token()?;
        Ok(Self { lex, cur })
    }

    /// Parses the entire input as a program: a sequence of and-or lists
    /// separated by `;`, `&`, or newlines. Returns an empty list if input
    /// is empty.
    pub fn parse_program(&mut self) -> Result<Cmd, LexError> {
        self.skip_newlines()?;
        let mut items: Vec<(Cmd, Sep)> = Vec::new();
        while !matches!(self.cur, Token::Eof) {
            let cmd = self.parse_and_or()?;
            let sep = match &self.cur {
                Token::Amp => {
                    self.advance()?;
                    Sep::Bg
                }
                Token::Semi | Token::Newline => {
                    self.advance()?;
                    Sep::Seq
                }
                Token::Eof => Sep::Seq,
                tok => return Err(LexError::new(alloc::format!("unexpected token: {:?}", tok))),
            };
            items.push((cmd, sep));
            self.skip_newlines()?;
        }
        if items.len() == 1 && items[0].1 == Sep::Seq {
            Ok(items.pop().unwrap().0)
        } else if items.is_empty() {
            Ok(Cmd::Empty)
        } else {
            Ok(Cmd::List(items))
        }
    }

    /// Parses one and-or chain (`a && b || c`).
    fn parse_and_or(&mut self) -> Result<Cmd, LexError> {
        let mut left = self.parse_pipeline()?;
        loop {
            let op = match &self.cur {
                Token::AndAnd => AndOrOp::And,
                Token::OrOr => AndOrOp::Or,
                _ => break,
            };
            self.advance()?;
            self.skip_newlines()?;
            let right = self.parse_pipeline()?;
            left = Cmd::AndOr {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    /// Parses one pipeline (`! cmd | cmd | cmd`).
    fn parse_pipeline(&mut self) -> Result<Cmd, LexError> {
        let mut negated = false;
        if let Token::Word(w) = &self.cur {
            if word_is_literal_keyword(w, "!") {
                negated = true;
                self.advance()?;
            }
        }
        let first = self.parse_command()?;
        // No `|` follows and no leading `!`: avoid wrapping a single command
        // in a one-stage Pipeline node.
        if !matches!(self.cur, Token::Pipe) && !negated {
            return Ok(first);
        }
        let mut parts = alloc::vec![first];
        while matches!(self.cur, Token::Pipe) {
            self.advance()?;
            self.skip_newlines()?;
            parts.push(self.parse_command()?);
        }
        Ok(Cmd::Pipeline { negated, parts })
    }

    /// Parses one command — simple, compound, or function definition.
    fn parse_command(&mut self) -> Result<Cmd, LexError> {
        // Look at the current token: keywords start compound forms.
        if let Token::Word(w) = &self.cur {
            if let Some(kw) = word_as_reserved(w) {
                match kw {
                    "if" => return self.parse_if(),
                    "while" => return self.parse_while(false),
                    "until" => return self.parse_while(true),
                    "for" => return self.parse_for(),
                    "{" => return self.parse_group(),
                    _ => {}
                }
            }
        }
        if matches!(self.cur, Token::LParen) {
            return self.parse_subshell();
        }
        self.parse_simple()
    }

    /// Parses a simple command — optional assignments, then words and
    /// redirections in any order until a terminator. Also recognises a
    /// function definition: `NAME ( ) compound_command`.
    fn parse_simple(&mut self) -> Result<Cmd, LexError> {
        let mut assigns: Vec<Assignment> = Vec::new();
        let mut words: Vec<Word> = Vec::new();
        let mut redirs: Vec<Redir> = Vec::new();
        let mut pending_fd: Option<i32> = None;
        let mut had_word = false;

        loop {
            match &self.cur {
                Token::IoNumber(n) => {
                    pending_fd = Some(*n);
                    self.advance()?;
                }
                Token::Less
                | Token::Greater
                | Token::DGreater
                | Token::LessAnd
                | Token::GreaterAnd
                | Token::AmpGreater
                | Token::Clobber => {
                    let op = redir_op_for(&self.cur);
                    self.advance()?;
                    let tgt = self.expect_word("redirection target")?;
                    let target = match op {
                        RedirOp::DupIn | RedirOp::DupOut => RedirTarget::Fd(tgt),
                        _ => RedirTarget::File(tgt),
                    };
                    redirs.push(Redir {
                        fd: pending_fd.take(),
                        op,
                        target,
                    });
                }
                Token::Word(_) => {
                    // Take the word; check for assignment or function def.
                    let w = if let Token::Word(w) = core::mem::replace(&mut self.cur, Token::Eof) {
                        w
                    } else {
                        unreachable!()
                    };
                    // Refill `self.cur`.
                    self.cur = self.lex.next_token()?;

                    // Function definition?  `name ( ) compound`.
                    if !had_word && assigns.is_empty() && matches!(self.cur, Token::LParen) {
                        if let Some(name) = word_as_simple_name(&w) {
                            // Consume `(` `)`.
                            self.advance()?;
                            if !matches!(self.cur, Token::RParen) {
                                return Err(LexError::new("expected ')' in function definition"));
                            }
                            self.advance()?;
                            self.skip_newlines()?;
                            let body = self.parse_command()?;
                            return Ok(Cmd::FuncDef {
                                name,
                                body: Box::new(body),
                            });
                        }
                    }

                    // Pre-command assignment?
                    if !had_word {
                        if let Some((name, value)) = word_as_assignment(&w) {
                            assigns.push(Assignment { name, value });
                            continue;
                        }
                    }

                    words.push(w);
                    had_word = true;
                }
                _ => break,
            }
        }

        Ok(Cmd::Simple(SimpleCmd {
            assigns,
            words,
            redirs,
        }))
    }

    fn parse_if(&mut self) -> Result<Cmd, LexError> {
        self.advance()?; // skip `if`
        let cond = self.parse_compound_list(&["then"])?;
        self.expect_keyword("then")?;
        let then = self.parse_compound_list(&["elif", "else", "fi"])?;
        let mut elifs: Vec<(Cmd, Cmd)> = Vec::new();
        while self.peek_keyword("elif") {
            self.advance()?;
            let c = self.parse_compound_list(&["then"])?;
            self.expect_keyword("then")?;
            let b = self.parse_compound_list(&["elif", "else", "fi"])?;
            elifs.push((c, b));
        }
        let els = if self.peek_keyword("else") {
            self.advance()?;
            Some(Box::new(self.parse_compound_list(&["fi"])?))
        } else {
            None
        };
        self.expect_keyword("fi")?;
        let redirs = self.parse_trailing_redirs()?;
        Ok(Cmd::If {
            cond: Box::new(cond),
            then: Box::new(then),
            elifs,
            els,
            redirs,
        })
    }

    fn parse_while(&mut self, until: bool) -> Result<Cmd, LexError> {
        self.advance()?;
        let cond = self.parse_compound_list(&["do"])?;
        self.expect_keyword("do")?;
        let body = self.parse_compound_list(&["done"])?;
        self.expect_keyword("done")?;
        let redirs = self.parse_trailing_redirs()?;
        Ok(Cmd::While {
            cond: Box::new(cond),
            body: Box::new(body),
            until,
            redirs,
        })
    }

    fn parse_for(&mut self) -> Result<Cmd, LexError> {
        self.advance()?; // skip `for`
        // Variable name (must be a bare word).
        let name = match &self.cur {
            Token::Word(w) => word_as_simple_name(w)
                .ok_or_else(|| LexError::new("expected variable name after `for`"))?,
            _ => return Err(LexError::new("expected variable name after `for`")),
        };
        self.advance()?;
        // Optional newlines, then optional `in WORDS`, then `;` or newline,
        // then `do BODY done`.
        self.skip_newlines()?;
        let words = if self.peek_keyword("in") {
            self.advance()?;
            let mut ws: Vec<Word> = Vec::new();
            while let Token::Word(w) = &self.cur {
                if word_is_literal_keyword(w, "do") {
                    break;
                }
                let w = w.clone();
                ws.push(w);
                self.advance()?;
            }
            // Optional `;` or newline before `do`.
            match &self.cur {
                Token::Semi | Token::Newline => {
                    self.advance()?;
                    self.skip_newlines()?;
                }
                _ => {}
            }
            Some(ws)
        } else {
            None
        };
        self.expect_keyword("do")?;
        let body = self.parse_compound_list(&["done"])?;
        self.expect_keyword("done")?;
        let redirs = self.parse_trailing_redirs()?;
        Ok(Cmd::For {
            var: name,
            words,
            body: Box::new(body),
            redirs,
        })
    }

    fn parse_group(&mut self) -> Result<Cmd, LexError> {
        self.advance()?; // `{`
        let body = self.parse_compound_list(&["}"])?;
        self.expect_keyword("}")?;
        let redirs = self.parse_trailing_redirs()?;
        Ok(Cmd::Group {
            body: Box::new(body),
            redirs,
        })
    }

    fn parse_subshell(&mut self) -> Result<Cmd, LexError> {
        self.advance()?; // `(`
        self.skip_newlines()?;
        let mut items: Vec<(Cmd, Sep)> = Vec::new();
        while !matches!(self.cur, Token::RParen) {
            let cmd = self.parse_and_or()?;
            let sep = match &self.cur {
                Token::Amp => {
                    self.advance()?;
                    Sep::Bg
                }
                Token::Semi | Token::Newline => {
                    self.advance()?;
                    Sep::Seq
                }
                Token::RParen => Sep::Seq,
                _ => return Err(LexError::new("expected `;`, `&`, newline, or `)`")),
            };
            items.push((cmd, sep));
            self.skip_newlines()?;
        }
        self.advance()?; // `)`
        let redirs = self.parse_trailing_redirs()?;
        let body = if items.len() == 1 && items[0].1 == Sep::Seq {
            items.pop().unwrap().0
        } else {
            Cmd::List(items)
        };
        Ok(Cmd::Subshell {
            body: Box::new(body),
            redirs,
        })
    }

    /// Parses a body of statements until one of the `terminators` keywords
    /// is next (and stays put — the caller consumes the terminator). Used
    /// inside `if/then/else/elif/fi`, `while/do/done`, `for/do/done`, `{ }`.
    fn parse_compound_list(&mut self, terminators: &[&str]) -> Result<Cmd, LexError> {
        self.skip_newlines()?;
        let mut items: Vec<(Cmd, Sep)> = Vec::new();
        loop {
            if self.peek_any_keyword(terminators) || matches!(self.cur, Token::Eof) {
                break;
            }
            let cmd = self.parse_and_or()?;
            let sep = match &self.cur {
                Token::Amp => {
                    self.advance()?;
                    Sep::Bg
                }
                Token::Semi | Token::Newline => {
                    self.advance()?;
                    Sep::Seq
                }
                _ => Sep::Seq,
            };
            items.push((cmd, sep));
            self.skip_newlines()?;
        }
        if items.len() == 1 && items[0].1 == Sep::Seq {
            Ok(items.pop().unwrap().0)
        } else if items.is_empty() {
            Ok(Cmd::Empty)
        } else {
            Ok(Cmd::List(items))
        }
    }

    fn parse_trailing_redirs(&mut self) -> Result<Vec<Redir>, LexError> {
        let mut redirs = Vec::new();
        let mut pending_fd: Option<i32> = None;
        loop {
            match &self.cur {
                Token::IoNumber(n) => {
                    pending_fd = Some(*n);
                    self.advance()?;
                }
                Token::Less
                | Token::Greater
                | Token::DGreater
                | Token::LessAnd
                | Token::GreaterAnd
                | Token::AmpGreater
                | Token::Clobber => {
                    let op = redir_op_for(&self.cur);
                    self.advance()?;
                    let tgt = self.expect_word("redirection target")?;
                    let target = match op {
                        RedirOp::DupIn | RedirOp::DupOut => RedirTarget::Fd(tgt),
                        _ => RedirTarget::File(tgt),
                    };
                    redirs.push(Redir {
                        fd: pending_fd.take(),
                        op,
                        target,
                    });
                }
                _ => break,
            }
        }
        Ok(redirs)
    }

    fn expect_word(&mut self, what: &str) -> Result<Word, LexError> {
        match core::mem::replace(&mut self.cur, Token::Eof) {
            Token::Word(w) => {
                self.cur = self.lex.next_token()?;
                Ok(w)
            }
            other => {
                self.cur = other;
                Err(LexError::new(alloc::format!("expected {}", what)))
            }
        }
    }

    fn expect_keyword(&mut self, kw: &str) -> Result<(), LexError> {
        if self.peek_keyword(kw) {
            self.advance()?;
            Ok(())
        } else {
            Err(LexError::new(alloc::format!("expected `{}`", kw)))
        }
    }

    fn peek_keyword(&self, kw: &str) -> bool {
        match &self.cur {
            Token::Word(w) => word_is_literal_keyword(w, kw),
            _ => false,
        }
    }

    fn peek_any_keyword(&self, kws: &[&str]) -> bool {
        kws.iter().any(|k| self.peek_keyword(k))
    }

    fn skip_newlines(&mut self) -> Result<(), LexError> {
        while matches!(self.cur, Token::Newline) {
            self.advance()?;
        }
        Ok(())
    }

    fn advance(&mut self) -> Result<(), LexError> {
        self.cur = self.lex.next_token()?;
        Ok(())
    }
}

fn redir_op_for(tok: &Token) -> RedirOp {
    match tok {
        Token::Less => RedirOp::In,
        Token::Greater => RedirOp::Out,
        Token::DGreater => RedirOp::Append,
        Token::LessAnd => RedirOp::DupIn,
        Token::GreaterAnd => RedirOp::DupOut,
        Token::AmpGreater => RedirOp::OutBoth,
        Token::Clobber => RedirOp::Clobber,
        _ => unreachable!(),
    }
}

/// Returns `true` if `w` is a single unquoted literal exactly equal to
/// `kw` — the only form in which a reserved word is recognised.
fn word_is_literal_keyword(w: &Word, kw: &str) -> bool {
    matches!(w.0.as_slice(), [Seg::Lit(s)] if s == kw)
}

/// If `w` is a bare-name literal, return it; used for the reserved-word
/// keyword lookup and for the LHS of function definitions and assignments.
fn word_as_simple_name(w: &Word) -> Option<String> {
    if let [Seg::Lit(s)] = w.0.as_slice() {
        if is_valid_name(s) {
            return Some(s.clone());
        }
    }
    None
}

fn is_valid_name(s: &str) -> bool {
    let mut bytes = s.bytes();
    match bytes.next() {
        Some(c) if c.is_ascii_alphabetic() || c == b'_' => {}
        _ => return false,
    }
    bytes.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

fn word_as_reserved(w: &Word) -> Option<&'static str> {
    if let [Seg::Lit(s)] = w.0.as_slice() {
        match s.as_str() {
            "if" => Some("if"),
            "then" => Some("then"),
            "else" => Some("else"),
            "elif" => Some("elif"),
            "fi" => Some("fi"),
            "while" => Some("while"),
            "until" => Some("until"),
            "do" => Some("do"),
            "done" => Some("done"),
            "for" => Some("for"),
            "in" => Some("in"),
            "{" => Some("{"),
            "}" => Some("}"),
            "!" => Some("!"),
            _ => None,
        }
    } else {
        None
    }
}

/// Returns `(name, value)` if the literal-prefixed word looks like a
/// `NAME=VALUE` assignment. Only the bytes before the first `=` need be a
/// valid name; the value can include further expansion segments.
fn word_as_assignment(w: &Word) -> Option<(String, Word)> {
    let first = w.0.first()?;
    let Seg::Lit(text) = first else {
        return None;
    };
    let eq = text.find('=')?;
    let (name, rest) = text.split_at(eq);
    if name.is_empty() || !is_valid_name(name) {
        return None;
    }
    let rest_lit = rest[1..].to_string();
    let mut value_segs: Vec<Seg> = Vec::new();
    if !rest_lit.is_empty() {
        value_segs.push(Seg::Lit(rest_lit));
    }
    value_segs.extend(w.0.iter().skip(1).cloned());
    Some((name.to_string(), Word(value_segs)))
}
