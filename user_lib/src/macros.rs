//! Print macros aligned with `std::macros`.
//!
//! These mirror the [`std`] versions: `print!`/`println!` write to
//! [`crate::io::stdout`], `eprint!`/`eprintln!` write to [`crate::io::stderr`],
//! and `dbg!` reports a `&str` representation of an expression to standard
//! error along with its file and line number.

/// Prints to standard output.
///
/// Equivalent to [`println!`] without a trailing newline.
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::io::_print(::core::format_args!($($arg)*))
    };
}

/// Prints to standard output, with a newline.
#[macro_export]
macro_rules! println {
    () => {
        $crate::print!("\n")
    };
    ($($arg:tt)*) => {
        $crate::io::_print(::core::format_args!("{}\n", ::core::format_args!($($arg)*)))
    };
}

/// Prints to standard error.
///
/// Equivalent to [`eprintln!`] without a trailing newline.
#[macro_export]
macro_rules! eprint {
    ($($arg:tt)*) => {
        $crate::io::_eprint(::core::format_args!($($arg)*))
    };
}

/// Prints to standard error, with a newline.
#[macro_export]
macro_rules! eprintln {
    () => {
        $crate::eprint!("\n")
    };
    ($($arg:tt)*) => {
        $crate::io::_eprint(::core::format_args!("{}\n", ::core::format_args!($($arg)*)))
    };
}

/// Prints and returns the value of a given expression for quick and dirty
/// debugging.
///
/// Output goes to standard error and is annotated with the source location.
#[macro_export]
macro_rules! dbg {
    () => {
        $crate::eprintln!("[{}:{}:{}]", ::core::file!(), ::core::line!(), ::core::column!())
    };
    ($val:expr $(,)?) => {
        match $val {
            tmp => {
                $crate::eprintln!(
                    "[{}:{}:{}] {} = {:#?}",
                    ::core::file!(),
                    ::core::line!(),
                    ::core::column!(),
                    ::core::stringify!($val),
                    &tmp,
                );
                tmp
            }
        }
    };
    ($($val:expr),+ $(,)?) => {
        ($($crate::dbg!($val)),+,)
    };
}
