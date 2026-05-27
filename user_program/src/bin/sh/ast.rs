//! Abstract syntax tree produced by the parser and consumed by the executor.
//!
//! A `Word` is the unit of input that becomes one or more arguments after
//! expansion. A `Cmd` is a single executable construct — a simple command,
//! a pipeline, a control-flow form, etc. Lists of commands separated by
//! `;` / `&` / newlines are represented by `Cmd::List`.

use alloc::{boxed::Box, string::String, vec::Vec};

/// One word in the source — accumulated as a sequence of segments so that
/// expansion can respect quoting rules (only unquoted segments are subject
/// to field splitting and pathname expansion).
#[derive(Clone, Debug)]
pub struct Word(pub Vec<Seg>);

/// A single segment inside a `Word`.
#[derive(Clone, Debug)]
pub enum Seg {
    /// Unquoted literal text. May contain glob metacharacters.
    Lit(String),
    /// Text from a single-quoted region — no further expansion at all.
    SQuoted(String),
    /// Sequence of segments from a double-quoted region. Variables expand,
    /// but field splitting and globbing are suppressed.
    DQuoted(Vec<Seg>),
    /// `$name` or `${name}`. The optional `ParamOp` carries `:-` / `:=`
    /// style suffix handling.
    Param {
        name: String,
        op: Option<ParamOp>,
        arg: Option<Box<Word>>,
        /// `${#name}` form.
        length: bool,
    },
    /// `$(...)` or `` `...` `` — raw source text to be re-parsed at run time.
    CmdSub(String),
    /// `$((expr))` — raw arithmetic expression source.
    Arith(String),
    /// `~` at the start of a word, optionally followed by a user name.
    Tilde(String),
}

/// Parameter expansion operator, mirroring POSIX `${var:-word}` family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamOp {
    /// `${var-word}` — use `word` if unset.
    DashUnset,
    /// `${var:-word}` — use `word` if unset or empty.
    DashNull,
    /// `${var=word}` — assign `word` to `var` if unset, then expand to it.
    EqUnset,
    /// `${var:=word}` — assign `word` to `var` if unset or empty.
    EqNull,
    /// `${var+word}` — use `word` if set.
    PlusUnset,
    /// `${var:+word}` — use `word` if set and non-empty.
    PlusNull,
    /// `${var?word}` — error with `word` if unset.
    QmarkUnset,
    /// `${var:?word}` — error with `word` if unset or empty.
    QmarkNull,
}

/// One redirection attached to a command.
#[derive(Clone, Debug)]
pub struct Redir {
    /// File descriptor being redirected; `None` means the operator's default
    /// (0 for `<`, 1 for `>`/`>>`/`&>`).
    pub fd: Option<i32>,
    pub op: RedirOp,
    pub target: RedirTarget,
}

#[derive(Clone, Debug)]
pub enum RedirTarget {
    /// `> FILE` etc. — target is a path word.
    File(Word),
    /// `>&N` / `<&N` — target is another fd (or `-` to close).
    Fd(Word),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RedirOp {
    /// `<` — open for reading, dup to fd.
    In,
    /// `>` — open for writing (truncate).
    Out,
    /// `>>` — open for writing (append).
    Append,
    /// `>|` — same as `>` (forcefully truncate); we treat noclobber as off.
    Clobber,
    /// `>&` — duplicate fd from operand.
    DupOut,
    /// `<&` — duplicate fd from operand.
    DupIn,
    /// `&>` — redirect both stdout and stderr to a file.
    OutBoth,
}

/// A simple command: pre-command assignments, words, and redirections.
#[derive(Clone, Debug, Default)]
pub struct SimpleCmd {
    /// `KEY=value` prefixes attached to this command (e.g. `FOO=bar cmd`).
    pub assigns: Vec<Assignment>,
    /// The command name and arguments, before expansion.
    pub words: Vec<Word>,
    /// Redirections, in source order.
    pub redirs: Vec<Redir>,
}

/// `NAME=word` form: the value is a full `Word` so it can contain
/// expansions.
#[derive(Clone, Debug)]
pub struct Assignment {
    pub name: String,
    pub value: Word,
}

/// Operator joining two pipelines in an `&&` / `||` chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AndOrOp {
    /// `&&` — run RHS only if LHS exit status was zero.
    And,
    /// `||` — run RHS only if LHS exit status was non-zero.
    Or,
}

/// Separator between two list items.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sep {
    /// `;` or newline — run sequentially, wait for completion.
    Seq,
    /// `&` — run in background, don't wait.
    Bg,
}

/// Top-level command form. Self-describing — recurses through itself for
/// compound forms.
#[derive(Clone, Debug)]
pub enum Cmd {
    /// `cmd arg arg`.
    Simple(SimpleCmd),
    /// `a | b | c` with optional leading `!` negation.
    Pipeline { negated: bool, parts: Vec<Cmd> },
    /// `lhs && rhs` or `lhs || rhs`.
    AndOr {
        left: Box<Cmd>,
        op: AndOrOp,
        right: Box<Cmd>,
    },
    /// `cmd1; cmd2 & cmd3` — items with their trailing separator.
    List(Vec<(Cmd, Sep)>),
    /// `(...)` — run in a subshell.
    Subshell { body: Box<Cmd>, redirs: Vec<Redir> },
    /// `{ ...; }` — group, runs in the current shell.
    Group { body: Box<Cmd>, redirs: Vec<Redir> },
    /// `if cond; then body; elif cond; then body; else body; fi`.
    If {
        cond: Box<Cmd>,
        then: Box<Cmd>,
        elifs: Vec<(Cmd, Cmd)>,
        els: Option<Box<Cmd>>,
        redirs: Vec<Redir>,
    },
    /// `while cond; do body; done` (or `until` if `negated` is set).
    While {
        cond: Box<Cmd>,
        body: Box<Cmd>,
        until: bool,
        redirs: Vec<Redir>,
    },
    /// `for var [in words]; do body; done`. When `words` is `None`, iterate
    /// over the positional parameters (`"$@"`).
    For {
        var: String,
        words: Option<Vec<Word>>,
        body: Box<Cmd>,
        redirs: Vec<Redir>,
    },
    /// `name() body` — defines a function in the current shell.
    FuncDef { name: String, body: Box<Cmd> },
    /// An empty command (used for empty input or `;` with nothing on the
    /// other side).
    Empty,
}
