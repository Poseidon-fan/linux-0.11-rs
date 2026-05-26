//! `pwd` — print the current working directory.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;

use user_lib::{
    env, fs,
    io::{self, Write},
    path::Path,
    process::ExitCode,
};
use user_program::cli::{Arg, Parser, program_name};

#[derive(Clone, Copy)]
enum Mode {
    Logical,
    Physical,
}

#[user_lib::main]
fn main() -> ExitCode {
    match run() {
        Ok(ignored_operands) => {
            if ignored_operands {
                user_lib::eprintln!("pwd: ignoring non-option arguments");
            }
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

fn run() -> Result<bool, ExitCode> {
    let (mode, ignored_operands) = parse_args()?;
    let cwd = match mode {
        Mode::Logical => logical_current_dir().unwrap_or_else(physical_current_dir),
        Mode::Physical => physical_current_dir(),
    };

    let mut out = io::stdout();
    out.write_all(cwd.as_bytes())
        .and_then(|()| out.write_all(b"\n"))
        .map_err(|err| {
            user_lib::eprintln!("pwd: {}", err);
            ExitCode::FAILURE
        })?;
    Ok(ignored_operands)
}

fn parse_args() -> Result<(Mode, bool), ExitCode> {
    let mut parser = Parser::from_env();
    let mut mode = Mode::Logical;
    let mut ignored_operands = false;

    while let Some(arg) = parser.next_arg().map_err(|err| {
        err.print_with_hint(&program_name());
        ExitCode::FAILURE
    })? {
        match arg {
            Arg::Short('L') => mode = Mode::Logical,
            Arg::Short('P') => mode = Mode::Physical,
            Arg::Short('h') => emit_and_exit(usage(), 0),
            Arg::Short(flag) => {
                user_lib::eprintln!("pwd: invalid option -- '{}'", flag);
                user_lib::eprintln!("Try 'pwd --help' for more information.");
                return Err(ExitCode::FAILURE);
            }
            Arg::Long(name) if name == "logical" => mode = Mode::Logical,
            Arg::Long(name) if name == "physical" => mode = Mode::Physical,
            Arg::Long(name) if name == "help" => emit_and_exit(usage(), 0),
            Arg::Long(name) if name == "version" => emit_and_exit(version_line(), 0),
            Arg::Long(name) => {
                user_lib::eprintln!("pwd: unrecognized option '--{}'", name);
                user_lib::eprintln!("Try 'pwd --help' for more information.");
                return Err(ExitCode::FAILURE);
            }
            Arg::Value(_) => {
                ignored_operands = true;
                break;
            }
        }
    }

    Ok((mode, ignored_operands))
}

fn logical_current_dir() -> Option<String> {
    let pwd = env::var("PWD").ok()?;
    if !Path::new(pwd.as_str()).is_absolute() {
        return None;
    }

    let pwd_metadata = fs::metadata(pwd.as_str()).ok()?;
    let dot_metadata = fs::metadata(".").ok()?;
    if same_file(&pwd_metadata, &dot_metadata) {
        Some(pwd)
    } else {
        None
    }
}

fn physical_current_dir() -> String {
    match env::current_dir() {
        Ok(path) => path.into_string(),
        Err(err) => {
            user_lib::eprintln!("pwd: {}", err);
            user_lib::process::exit(1);
        }
    }
}

fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.dev() == right.dev() && left.ino() == right.ino()
}

fn usage() -> &'static str {
    "Usage: pwd [OPTION]...\n\
     Print the full filename of the current working directory.\n\
     \n\
     Options:\n\
     \x20 -L, --logical    use PWD from the environment if valid\n\
     \x20 -P, --physical   avoid all symbolic links\n\
     \x20 -h, --help       show this help\n\
     \x20 --version        show version\n"
}

fn version_line() -> String {
    let mut out = program_name();
    out.push(' ');
    out.push_str(env!("CARGO_PKG_VERSION"));
    out.push('\n');
    out
}

fn emit_and_exit<T: AsRef<str>>(text: T, code: i32) -> ! {
    let mut out = io::stdout();
    let _ = out.write_all(text.as_ref().as_bytes());
    user_lib::process::exit(code)
}
