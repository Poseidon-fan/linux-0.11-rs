//! Reusable compiler-frontend toolkit: lexer and parser combinators.
//!
//! Build a [`lex::Lexer`] declaratively from `whitespace`, `keyword`,
//! `ident`, `int`, etc., then compose [`parse::Parser`]s with `seq`,
//! `choice`, `optional`, `many`, `sep_by`, `lazy`, and `pratt` to turn
//! the resulting token stream into a typed AST. Both modules are
//! `no_std`-friendly (they only depend on `alloc`).
//!
//! Bring everything into scope with a single glob:
//!
//! ```ignore
//! use user_program::compiler::{lex::*, parse::*};
//! ```

pub mod lex;
pub mod parse;
