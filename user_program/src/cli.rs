//! Command-line argument parsing.
//!
//! A small parser inspired by `lexopt`, plus a declarative macro that lets a
//! command describe its arguments as a plain struct and gets `--help`,
//! `--version`, usage rendering, and "try --help" hinting for free.
//!
//! ```ignore
//! use user_program::cli::cli_args;
//!
//! cli_args! {
//!     /// Concatenate FILE(s) to standard output.
//!     pub struct CatArgs {
//!         /// Number all output lines.
//!         pub number: bool        = ["-n", "--number"],
//!         /// Files to read; stdin if none given.
//!         pub files:  Vec<String> = [..] @ "FILE",
//!     }
//! }
//!
//! let args = CatArgs::parse_env_or_exit();
//! ```
//!
//! ### Spec syntax
//!
//! For each field:
//!
//! ```text
//! pub <field>: <Type> = <spec> [@ "<VALUE_NAME>"] [= <default_expr>]
//! ```
//!
//! - `<spec>` is `[..]` (positional sink) or `[ "-x", "--long", ... ]`.
//! - `@ "NAME"` overrides the value placeholder shown in usage.
//! - `= expr` sets a default value used when the flag is absent.
//!
//! Field types map to behaviour via [`FromCli`] / [`FromCliPositional`]: any
//! non-`bool` field implicitly takes a value, `Option<T>` lets a flag stay
//! unset, `Vec<String>` either captures positionals (with `[..]`) or
//! collects repeated options.
//!
//! ### Auto behaviour
//!
//! Every generated parser silently accepts `-h` / `--help` and `--version`.
//! - On `-h` / `--help` it writes the rendered usage to stdout and calls
//!   `process::exit(0)`.
//! - On `--version` it writes `"<program> <version>"` and exits.
//! - On a parse error, [`Self::parse_env_or_exit`] prints the error and
//!   `Try '<program> --help' for more information.` to stderr, then exits
//!   with status `2`.
//!
//! Callers that need finer control can use [`Self::parse_env`] instead, which
//! returns [`Error`] and never exits.

use alloc::{
    borrow::ToOwned,
    boxed::Box,
    string::{String, ToString},
    vec::Vec,
};
use core::{error, fmt, str::FromStr};

use user_lib::path::Path;

/// Re-export of the crate-root [`cli_args!`](crate::cli_args) macro under
/// the module path most users will reach for first.
pub use crate::cli_args;

// ---------------------------------------------------------------------------
// Arg
// ---------------------------------------------------------------------------

/// One parsed token yielded by [`Parser::next_arg`].
#[derive(Debug)]
pub enum Arg {
    /// A short flag like `-r`. Clusters (`-rfv`) yield one variant per
    /// character.
    Short(char),
    /// A long flag like `--recursive`, without the leading `--`.
    Long(String),
    /// A positional value, or any argument after `--`.
    Value(String),
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// An error returned from argument parsing.
#[derive(Debug)]
pub enum Error {
    /// A short flag that no field declared.
    UnknownShort(char),
    /// A long flag that no field declared.
    UnknownLong(String),
    /// A positional value with no sink declared.
    UnexpectedPositional(String),
    /// A flag that expects a value was given none.
    MissingValue {
        /// The flag name as the user wrote it (`-n`, `--lines`, …).
        flag: String,
    },
    /// A flag's value failed to parse into the field's type.
    InvalidValue {
        /// The flag name as the user wrote it.
        flag: String,
        /// The raw text the user supplied.
        value: String,
    },
}

impl Error {
    /// Prints `"<program>: <error>"` and a hint to try `--help` on stderr.
    pub fn print_with_hint(&self, program: &str) {
        user_lib::eprintln!("{}: {}", program, self);
        user_lib::eprintln!("Try '{} --help' for more information.", program);
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnknownShort(c) => write!(f, "unknown option: -{}", c),
            Error::UnknownLong(name) => write!(f, "unknown option: --{}", name),
            Error::UnexpectedPositional(v) => write!(f, "unexpected argument: {}", v),
            Error::MissingValue { flag } => write!(f, "{} requires a value", flag),
            Error::InvalidValue { flag, value } => {
                write!(f, "{}: invalid value: {}", flag, value)
            }
        }
    }
}

impl error::Error for Error {}

impl From<Error> for user_lib::io::Error {
    fn from(err: Error) -> Self {
        user_lib::io::Error::new(user_lib::io::ErrorKind::InvalidInput, Box::new(err))
    }
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Stateful, iterator-style argument parser.
///
/// Consume tokens one at a time with [`Parser::next_arg`]. When a flag is
/// returned that expects an inline value, fetch it with
/// [`Parser::value`].
pub struct Parser {
    remaining: Vec<String>,
    short_cluster: Option<(String, usize)>,
    long_value: Option<String>,
    last_flag: Option<String>,
    positional_only: bool,
}

impl Parser {
    /// Builds a parser over the current process's argv, skipping `argv[0]`.
    pub fn from_env() -> Self {
        Self::from_args(user_lib::env::args().skip(1))
    }

    /// Builds a parser over an arbitrary argument sequence.
    pub fn from_args<I, S>(args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut all: Vec<String> = args.into_iter().map(Into::into).collect();
        all.reverse();
        Self {
            remaining: all,
            short_cluster: None,
            long_value: None,
            last_flag: None,
            positional_only: false,
        }
    }

    /// Returns the next argument token, or `Ok(None)` at end of input.
    pub fn next_arg(&mut self) -> Result<Option<Arg>, Error> {
        if let Some((cluster, idx)) = self.short_cluster.take() {
            let bytes = cluster.as_bytes();
            if idx < bytes.len() {
                let ch = bytes[idx] as char;
                let next_idx = idx + 1;
                if next_idx < bytes.len() {
                    self.short_cluster = Some((cluster, next_idx));
                }
                self.last_flag = Some(format_short(ch));
                return Ok(Some(Arg::Short(ch)));
            }
        }

        let Some(raw) = self.remaining.pop() else {
            return Ok(None);
        };

        if self.positional_only {
            self.last_flag = None;
            return Ok(Some(Arg::Value(raw)));
        }

        if raw == "--" {
            self.positional_only = true;
            return self.next_arg();
        }

        if let Some(rest) = raw.strip_prefix("--") {
            if rest.is_empty() {
                self.positional_only = true;
                return self.next_arg();
            }
            let (name, value) = match rest.find('=') {
                Some(eq) => (rest[..eq].to_string(), Some(rest[eq + 1..].to_string())),
                None => (rest.to_string(), None),
            };
            self.last_flag = Some(format_long(&name));
            self.long_value = value;
            return Ok(Some(Arg::Long(name)));
        }

        if raw.len() > 1 && raw.starts_with('-') {
            let cluster = raw[1..].to_string();
            let bytes = cluster.as_bytes();
            let ch = bytes[0] as char;
            if bytes.len() > 1 {
                self.short_cluster = Some((cluster, 1));
            }
            self.last_flag = Some(format_short(ch));
            return Ok(Some(Arg::Short(ch)));
        }

        self.last_flag = None;
        Ok(Some(Arg::Value(raw)))
    }

    /// Fetches the value associated with the most recently yielded flag.
    pub fn value(&mut self) -> Result<String, Error> {
        if let Some(v) = self.long_value.take() {
            return Ok(v);
        }
        if let Some((cluster, idx)) = self.short_cluster.take() {
            return Ok(cluster[idx..].to_string());
        }
        self.remaining.pop().ok_or_else(|| Error::MissingValue {
            flag: self.last_flag.clone().unwrap_or_default(),
        })
    }

    /// Returns the most recently yielded flag spelled the way the user
    /// wrote it (`-n`, `--lines`, …). Empty before the first flag.
    pub fn last_flag_name(&self) -> String {
        self.last_flag.clone().unwrap_or_default()
    }

    /// Consumes the parser and returns the unparsed raw arguments in their
    /// original order.
    ///
    /// This is useful for commands such as `env`, where option parsing stops
    /// at the first non-option operand and all following tokens belong to the
    /// child command, even if they start with `-`.
    pub fn into_remaining_values(mut self) -> Vec<String> {
        self.short_cluster = None;
        self.long_value = None;
        self.remaining.reverse();
        self.remaining
    }
}

fn format_short(c: char) -> String {
    let mut s = String::with_capacity(2);
    s.push('-');
    s.push(c);
    s
}

fn format_long(name: &str) -> String {
    let mut s = String::with_capacity(name.len() + 2);
    s.push_str("--");
    s.push_str(name);
    s
}

/// Returns the basename of `argv[0]`, falling back to `"<program>"`.
pub fn program_name() -> String {
    let argv0 = user_lib::env::args().next().unwrap_or_default();
    if argv0.is_empty() {
        return "<program>".to_string();
    }
    Path::new(&argv0)
        .file_name()
        .unwrap_or(argv0.as_str())
        .to_owned()
}

// ---------------------------------------------------------------------------
// FromCli / FromCliPositional traits + impls
// ---------------------------------------------------------------------------

/// How a struct field reacts when its flag fires.
pub trait FromCli {
    /// Called once per flag hit.
    fn set_from_flag(&mut self, parser: &mut Parser) -> Result<(), Error>;
}

/// How a struct field acts as the destination of positional arguments.
pub trait FromCliPositional {
    /// Called once per positional value.
    fn push_positional(&mut self, value: String) -> Result<(), Error>;
}

impl FromCli for bool {
    fn set_from_flag(&mut self, _parser: &mut Parser) -> Result<(), Error> {
        *self = true;
        Ok(())
    }
}

impl FromCli for String {
    fn set_from_flag(&mut self, parser: &mut Parser) -> Result<(), Error> {
        *self = parser.value()?;
        Ok(())
    }
}

impl FromCliPositional for String {
    fn push_positional(&mut self, value: String) -> Result<(), Error> {
        *self = value;
        Ok(())
    }
}

impl FromCli for Option<String> {
    fn set_from_flag(&mut self, parser: &mut Parser) -> Result<(), Error> {
        *self = Some(parser.value()?);
        Ok(())
    }
}

impl FromCliPositional for Option<String> {
    fn push_positional(&mut self, value: String) -> Result<(), Error> {
        *self = Some(value);
        Ok(())
    }
}

impl FromCli for Vec<String> {
    fn set_from_flag(&mut self, parser: &mut Parser) -> Result<(), Error> {
        self.push(parser.value()?);
        Ok(())
    }
}

impl FromCliPositional for Vec<String> {
    fn push_positional(&mut self, value: String) -> Result<(), Error> {
        self.push(value);
        Ok(())
    }
}

macro_rules! impl_fromcli_via_fromstr {
    ($($ty:ty),* $(,)?) => {
        $(
            impl FromCli for $ty {
                fn set_from_flag(&mut self, parser: &mut Parser) -> Result<(), Error> {
                    let flag = parser.last_flag_name();
                    let raw = parser.value()?;
                    *self = <$ty as FromStr>::from_str(&raw).map_err(|_| {
                        Error::InvalidValue { flag, value: raw }
                    })?;
                    Ok(())
                }
            }
        )*
    };
}

impl_fromcli_via_fromstr!(u8, u16, u32, usize, i8, i16, i32, isize);

// ---------------------------------------------------------------------------
// Help model + renderer
// ---------------------------------------------------------------------------

/// One row in the rendered usage table.
///
/// `aliases` keeps the raw spec strings (`"-n"`, `"--number"`) untouched
/// so the renderer can classify and order them itself. `is_positional`
/// switches the row between the *Options* and *Arguments* sections of
/// the usage output.
pub struct HelpEntry {
    pub aliases: &'static [&'static str],
    pub value_name: Option<&'static str>,
    pub help: &'static str,
    pub is_positional: bool,
    pub takes_value: bool,
}

const OPTIONS_PAD: usize = 22;

/// Render the full `--help` message.
pub fn render_usage(
    program: &str,
    description: &str,
    version: &str,
    entries: &[HelpEntry],
) -> String {
    let mut out = String::new();

    // --- Usage line ---
    out.push_str("Usage: ");
    out.push_str(program);
    if entries.iter().any(|e| !e.is_positional) {
        out.push_str(" [OPTIONS]");
    }
    for entry in entries.iter().filter(|e| e.is_positional) {
        out.push(' ');
        out.push('[');
        out.push_str(entry.value_name.unwrap_or("VALUE"));
        out.push_str("]...");
    }
    out.push('\n');

    if !description.is_empty() {
        out.push('\n');
        out.push_str(description.trim());
        out.push('\n');
    }

    // --- Arguments section ---
    let mut positional_iter = entries.iter().filter(|e| e.is_positional).peekable();
    if positional_iter.peek().is_some() {
        out.push_str("\nArguments:\n");
        for entry in entries.iter().filter(|e| e.is_positional) {
            let label = bracketed(entry.value_name.unwrap_or("VALUE"));
            push_row(&mut out, &label, entry.help);
        }
    }

    // --- Options section ---
    out.push_str("\nOptions:\n");
    for entry in entries.iter().filter(|e| !e.is_positional) {
        let label = format_option_label(entry);
        push_row(&mut out, &label, entry.help);
    }
    // Built-in entries
    push_row(&mut out, "-h, --help", "Print help");
    if !version.is_empty() {
        push_row(&mut out, "    --version", "Print version");
    }

    out
}

fn format_option_label(entry: &HelpEntry) -> String {
    let mut shorts: Vec<&'static str> = Vec::new();
    let mut longs: Vec<&'static str> = Vec::new();
    for alias in entry.aliases {
        if alias.starts_with("--") {
            longs.push(alias);
        } else if alias.starts_with('-') {
            shorts.push(alias);
        }
    }

    let mut label = String::new();
    if let Some(first) = shorts.first() {
        label.push_str(first);
        if !longs.is_empty() {
            label.push_str(", ");
        }
    } else {
        label.push_str("    ");
    }
    if let Some(first) = longs.first() {
        label.push_str(first);
    }
    if entry.takes_value {
        label.push(' ');
        label.push('<');
        label.push_str(entry.value_name.unwrap_or("VALUE"));
        label.push('>');
    }
    label
}

fn bracketed(name: &str) -> String {
    let mut s = String::with_capacity(name.len() + 2);
    s.push('[');
    s.push_str(name);
    s.push(']');
    s
}

fn push_row(out: &mut String, label: &str, help: &str) {
    out.push_str("  ");
    out.push_str(label);
    if help.is_empty() {
        out.push('\n');
        return;
    }
    if label.len() < OPTIONS_PAD {
        for _ in label.len()..OPTIONS_PAD {
            out.push(' ');
        }
    } else {
        out.push('\n');
        for _ in 0..OPTIONS_PAD + 2 {
            out.push(' ');
        }
    }
    out.push_str(help.trim());
    out.push('\n');
}

// ---------------------------------------------------------------------------
// Hidden runtime helpers
// ---------------------------------------------------------------------------

#[doc(hidden)]
pub fn __match_short(flag: &str, c: char) -> bool {
    let bytes = flag.as_bytes();
    bytes.len() == 2 && bytes[0] == b'-' && bytes[1] != b'-' && (bytes[1] as char) == c
}

#[doc(hidden)]
pub fn __match_long(flag: &str, name: &str) -> bool {
    matches!(flag.strip_prefix("--"), Some(rest) if rest == name)
}

/// Joins consecutive doc-comment lines into one help string, trimming the
/// single leading space that `///` injects.
#[doc(hidden)]
pub fn __join_doc(lines: &[&'static str]) -> String {
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let trimmed = line.strip_prefix(' ').unwrap_or(line);
        out.push_str(trimmed);
    }
    out
}

/// Prints `text` to stdout and exits with status 0.
#[doc(hidden)]
pub fn __emit_and_exit(text: &str) -> ! {
    use user_lib::io::Write;
    let mut out = user_lib::io::stdout();
    let _ = out.write_all(text.as_bytes());
    if !text.ends_with('\n') {
        let _ = out.write_all(b"\n");
    }
    user_lib::process::exit(0);
}

// ---------------------------------------------------------------------------
// Internal macros driven by `cli_args!`
// ---------------------------------------------------------------------------

#[doc(hidden)]
#[macro_export]
macro_rules! __cli_is_positional {
    ([..]) => {
        true
    };
    ([$($_flag:literal),* $(,)?]) => {
        false
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __cli_matches_short {
    ([..], $c:expr) => { false };
    ([$($flag:literal),* $(,)?], $c:expr) => {{
        let __ch: ::core::primitive::char = $c;
        false $(|| $crate::cli::__match_short($flag, __ch))*
    }};
}

#[doc(hidden)]
#[macro_export]
macro_rules! __cli_matches_long {
    ([..], $n:expr) => { false };
    ([$($flag:literal),* $(,)?], $n:expr) => {{
        let __name: &::core::primitive::str = $n;
        false $(|| $crate::cli::__match_long($flag, __name))*
    }};
}

#[doc(hidden)]
#[macro_export]
macro_rules! __cli_run_positional {
    ([..], $field:expr, $value:expr) => {
        $crate::cli::FromCliPositional::push_positional(&mut $field, $value)?
    };
    ([$($_flag:literal),* $(,)?], $field:expr, $value:expr) => {
        // not a positional sink — no-op
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __cli_aliases_slice {
    ([..]) => { &[] as &[&'static ::core::primitive::str] };
    ([$($flag:literal),* $(,)?]) => { &[$($flag),*] as &[&'static ::core::primitive::str] };
}

/// `__cli_default_value!($ty)` → `Default::default()`.
/// `__cli_default_value!($ty, $expr)` → `$expr`.
#[doc(hidden)]
#[macro_export]
macro_rules! __cli_default_value {
    ($ty:ty) => {
        <$ty as ::core::default::Default>::default()
    };
    ($ty:ty, $expr:expr) => {
        $expr
    };
}

/// `__cli_takes_value!(bool)` → `false`; everything else → `true`.
/// Used by the help renderer; bool flags shouldn't show `<VALUE>`.
#[doc(hidden)]
#[macro_export]
macro_rules! __cli_takes_value {
    (bool) => {
        false
    };
    ($_other:ty) => {
        true
    };
}

// ---------------------------------------------------------------------------
// Main macro
// ---------------------------------------------------------------------------

/// Define a struct of command-line arguments together with `parse_env`,
/// `parse_env_or_exit`, and `usage` methods.
///
/// See the [module-level documentation](self) for syntax and supported
/// field types.
#[macro_export]
macro_rules! cli_args {
    (
        $(#[doc = $struct_doc:literal])*
        $vis:vis struct $name:ident {
            $(
                $(#[doc = $field_doc:literal])*
                $field_vis:vis $field:ident : $ty:ty = $spec:tt
                $(@ $value_name:literal)?
                $(= $default:expr)?
            ),* $(,)?
        }
    ) => {
        $(#[doc = $struct_doc])*
        $vis struct $name {
            $(
                $(#[doc = $field_doc])*
                $field_vis $field: $ty,
            )*
        }

        impl ::core::default::Default for $name {
            fn default() -> Self {
                Self {
                    $(
                        $field: $crate::__cli_default_value!($ty $(, $default)?),
                    )*
                }
            }
        }

        impl $name {
            /// Static description, built from the struct's doc comment.
            const __CLI_DESCRIPTION: &'static [&'static ::core::primitive::str] = &[
                $($struct_doc,)*
            ];

            /// Static option metadata for usage rendering.
            const __CLI_ENTRIES: &'static [&'static dyn ::core::ops::Fn() -> $crate::cli::HelpEntry] =
                &[];

            /// Parses this struct from the current process's argv.
            ///
            /// `-h`, `--help`, and `--version` are intercepted: they print
            /// and call `process::exit(0)` instead of returning.
            pub fn parse_env() -> ::core::result::Result<Self, $crate::cli::Error> {
                Self::parse_from(::user_lib::env::args().skip(1))
            }

            /// Parses argv, printing usage / version / errors directly to
            /// the appropriate stream and exiting on any of those paths.
            pub fn parse_env_or_exit() -> Self {
                match Self::parse_env() {
                    ::core::result::Result::Ok(args) => args,
                    ::core::result::Result::Err(err) => {
                        err.print_with_hint(&$crate::cli::program_name());
                        ::user_lib::process::exit(2);
                    }
                }
            }

            /// Parses this struct from an arbitrary argument sequence.
            pub fn parse_from<__I, __S>(
                __args: __I,
            ) -> ::core::result::Result<Self, $crate::cli::Error>
            where
                __I: ::core::iter::IntoIterator<Item = __S>,
                __S: ::core::convert::Into<::alloc::string::String>,
            {
                let mut __out: Self = ::core::default::Default::default();
                let mut __parser = $crate::cli::Parser::from_args(__args);

                while let ::core::option::Option::Some(__arg) = __parser.next_arg()? {
                    match __arg {
                        $crate::cli::Arg::Value(mut __v) => {
                            let mut __handled = false;
                            $(
                                if !__handled && $crate::__cli_is_positional!($spec) {
                                    $crate::__cli_run_positional!(
                                        $spec,
                                        __out.$field,
                                        ::core::mem::take(&mut __v)
                                    );
                                    __handled = true;
                                }
                            )*
                            if !__handled {
                                return ::core::result::Result::Err(
                                    $crate::cli::Error::UnexpectedPositional(__v),
                                );
                            }
                        }
                        $crate::cli::Arg::Short(__c) => {
                            let mut __handled = false;
                            $(
                                if !__handled && $crate::__cli_matches_short!($spec, __c) {
                                    <_ as $crate::cli::FromCli>::set_from_flag(
                                        &mut __out.$field,
                                        &mut __parser,
                                    )?;
                                    __handled = true;
                                }
                            )*
                            if !__handled && __c == 'h' {
                                $crate::cli::__emit_and_exit(&Self::usage());
                            }
                            if !__handled {
                                return ::core::result::Result::Err(
                                    $crate::cli::Error::UnknownShort(__c),
                                );
                            }
                        }
                        $crate::cli::Arg::Long(__name) => {
                            let mut __handled = false;
                            $(
                                if !__handled
                                    && $crate::__cli_matches_long!($spec, __name.as_str())
                                {
                                    <_ as $crate::cli::FromCli>::set_from_flag(
                                        &mut __out.$field,
                                        &mut __parser,
                                    )?;
                                    __handled = true;
                                }
                            )*
                            if !__handled && __name == "help" {
                                $crate::cli::__emit_and_exit(&Self::usage());
                            }
                            if !__handled && __name == "version" {
                                $crate::cli::__emit_and_exit(&Self::version_line());
                            }
                            if !__handled {
                                return ::core::result::Result::Err(
                                    $crate::cli::Error::UnknownLong(__name),
                                );
                            }
                        }
                    }
                }

                ::core::result::Result::Ok(__out)
            }

            /// Returns the rendered `--help` output.
            pub fn usage() -> ::alloc::string::String {
                let __entries: ::alloc::vec::Vec<$crate::cli::HelpEntry> = ::alloc::vec![
                    $(
                        $crate::cli::HelpEntry {
                            aliases: $crate::__cli_aliases_slice!($spec),
                            value_name: $crate::__cli_value_name!($ty, $spec $(, $value_name)?),
                            help: ::alloc::boxed::Box::leak(
                                $crate::cli::__join_doc(&[$($field_doc,)*]).into_boxed_str()
                            ),
                            is_positional: $crate::__cli_is_positional!($spec),
                            takes_value: $crate::__cli_field_takes_value!($ty, $spec),
                        },
                    )*
                ];
                let __description = $crate::cli::__join_doc(Self::__CLI_DESCRIPTION);
                let __version = Self::version_line();
                $crate::cli::render_usage(
                    &$crate::cli::program_name(),
                    &__description,
                    if __version.is_empty() { "" } else { __version.as_str() },
                    &__entries,
                )
            }

            /// Returns `"<program> <version>"` for `--version`.
            pub fn version_line() -> ::alloc::string::String {
                let mut __out = ::alloc::string::String::new();
                __out.push_str(&$crate::cli::program_name());
                __out.push(' ');
                __out.push_str(env!("CARGO_PKG_VERSION"));
                __out
            }
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __cli_field_takes_value {
    // Positional [..] doesn't show <VALUE> twice
    ($ty:ty, [..]) => {
        false
    };
    (bool, $spec:tt) => {
        false
    };
    ($_ty:ty, $spec:tt) => {
        true
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __cli_value_name {
    // Explicit @ "NAME"
    ($_ty:ty, $_spec:tt, $name:literal) => {
        ::core::option::Option::Some($name)
    };
    // Default by type (for non-bool fields)
    (bool, $_spec:tt) => {
        ::core::option::Option::None
    };
    ($_ty:ty, [..]) => {
        ::core::option::Option::Some("VALUE")
    };
    ($_ty:ty, $_spec:tt) => {
        ::core::option::Option::Some("VALUE")
    };
}
