//! `env` — print the environment or run a command in a modified environment.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

use user_lib::{
    env, fs,
    io::{self, Write},
    process::{self, Command, ExitCode},
};
use user_program::cli::{Arg, Error as CliError, Parser, cli_args};

cli_args! {
    /// Set each NAME to VALUE in the environment and run COMMAND. If no
    /// COMMAND is given, print the resulting environment.
    pub struct EnvArgs {
        /// Start with an empty environment.
        pub ignore_environment: bool        = ["-i", "--ignore-environment"],
        /// End each output item with NUL, not newline.
        pub zero:               bool        = ["-0", "--null"],
        /// Remove variable from the environment.
        pub unset:              Vec<String> = ["-u", "--unset"] @ "NAME",
        /// Change working directory to DIR.
        pub chdir:              Option<String> = ["-C", "--chdir"] @ "DIR",
        /// Pass ARG as argv[0] to COMMAND.
        pub argv0:              Option<String> = ["-a", "--argv0"] @ "ARG",
        /// Environment assignments followed by an optional command.
        pub operands:           Vec<String> = [..] @ "NAME=VALUE | COMMAND",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(message) => {
            user_lib::eprintln!("env: {}", message);
            ExitCode::from(125)
        }
    }
}

struct EnvOptions {
    args: EnvArgs,
    overrides: Vec<(String, String)>,
    command_args: Vec<String>,
}

/// Parses env arguments and either prints or executes with overrides.
fn run() -> Result<ExitCode, String> {
    let options = parse_env_or_exit();

    if let Some(dir) = options.args.chdir.as_deref() {
        fs::set_current_dir(dir).map_err(|err| format!("{}: {}", dir, err))?;
    }

    if options.command_args.is_empty() {
        print_environment(&options).map_err(|err| err.to_string())?;
        return Ok(ExitCode::SUCCESS);
    }

    execute_command(&options)
}

/// Parses `env`'s option prefix using the shared CLI tokeniser.
///
/// Unlike ordinary commands, parsing stops at the first non-option operand:
/// assignments are consumed by `env`, and the first remaining operand plus
/// every following token is the child command.
fn parse_env_or_exit() -> EnvOptions {
    match parse_env_options() {
        Ok(options) => options,
        Err(err) => {
            err.print_with_hint(&user_program::cli::program_name());
            process::exit(2);
        }
    }
}

fn parse_env_options() -> Result<EnvOptions, CliError> {
    let mut args = EnvArgs::default();
    let mut parser = Parser::from_env();

    while let Some(arg) = parser.next_arg()? {
        match arg {
            Arg::Short('i') => args.ignore_environment = true,
            Arg::Short('0') => args.zero = true,
            Arg::Short('u') => args.unset.push(parser.value()?),
            Arg::Short('C') => args.chdir = Some(parser.value()?),
            Arg::Short('a') => args.argv0 = Some(parser.value()?),
            Arg::Short('h') => emit_and_exit(&EnvArgs::usage()),
            Arg::Short(flag) => return Err(CliError::UnknownShort(flag)),
            Arg::Long(name) if name == "ignore-environment" => args.ignore_environment = true,
            Arg::Long(name) if name == "null" => args.zero = true,
            Arg::Long(name) if name == "unset" => args.unset.push(parser.value()?),
            Arg::Long(name) if name == "chdir" => args.chdir = Some(parser.value()?),
            Arg::Long(name) if name == "argv0" => args.argv0 = Some(parser.value()?),
            Arg::Long(name) if name == "help" => emit_and_exit(&EnvArgs::usage()),
            Arg::Long(name) if name == "version" => emit_and_exit(&EnvArgs::version_line()),
            Arg::Long(name) => return Err(CliError::UnknownLong(name)),
            Arg::Value(value) if value == "-" => args.ignore_environment = true,
            Arg::Value(value) => {
                args.operands.push(value);
                args.operands.extend(parser.into_remaining_values());
                break;
            }
        }
    }

    let mut options = EnvOptions {
        args,
        overrides: Vec::new(),
        command_args: Vec::new(),
    };
    split_operands(&mut options);
    Ok(options)
}

fn split_operands(options: &mut EnvOptions) {
    let operands = core::mem::take(&mut options.args.operands);
    let mut iter = operands.into_iter();

    while let Some(operand) = iter.next() {
        if let Some(eq) = operand.find('=') {
            options
                .overrides
                .push((operand[..eq].to_string(), operand[eq + 1..].to_string()));
        } else {
            options.command_args.push(operand);
            options.command_args.extend(iter);
            break;
        }
    }
}

/// Prints the resulting environment after applying requested overrides.
fn print_environment(options: &EnvOptions) -> user_lib::io::Result<()> {
    let terminator = if options.args.zero { 0 } else { b'\n' };
    let mut vars: Vec<(String, String)> = if options.args.ignore_environment {
        Vec::new()
    } else {
        env::vars().collect()
    };
    apply_unsets(&mut vars, &options.args.unset);
    apply_overrides(&mut vars, &options.overrides);

    let mut out = io::stdout();
    for (name, value) in vars {
        out.write_all(name.as_bytes())?;
        out.write_all(b"=")?;
        out.write_all(value.as_bytes())?;
        out.write_all(&[terminator])?;
    }
    Ok(())
}

/// Executes the requested command with the resulting environment.
fn execute_command(options: &EnvOptions) -> Result<ExitCode, String> {
    let Some(program) = options.command_args.first() else {
        return Ok(ExitCode::SUCCESS);
    };

    let mut command = Command::new(program.as_str());
    if let Some(arg0) = options.args.argv0.as_deref() {
        command.arg0(arg0);
    }
    if options.args.ignore_environment {
        command.env_clear();
    }
    for name in &options.args.unset {
        command.env_remove(name.as_str());
    }
    for (name, value) in &options.overrides {
        command.env(name.as_str(), value.as_str());
    }
    command.args(options.command_args.iter().skip(1).map(String::as_str));

    match command.status() {
        Ok(status) => Ok(status
            .code()
            .map(|code| ExitCode::from(code as u8))
            .unwrap_or(ExitCode::FAILURE)),
        Err(err) => {
            user_lib::eprintln!("env: {}: {}", program, err);
            Ok(ExitCode::from(127))
        }
    }
}

/// Removes requested variable names from the environment vector.
fn apply_unsets(vars: &mut Vec<(String, String)>, unsets: &[String]) {
    for name in unsets {
        vars.retain(|(existing, _)| existing != name);
    }
}

/// Applies assignment arguments using the same last-assignment-wins behavior
/// as traditional `env`.
fn apply_overrides(vars: &mut Vec<(String, String)>, overrides: &[(String, String)]) {
    for (name, value) in overrides {
        vars.retain(|(existing, _)| existing != name);
        vars.push((name.clone(), value.clone()));
    }
}

fn emit_and_exit(text: &str) -> ! {
    let mut out = io::stdout();
    let _ = out.write_all(text.as_bytes());
    if !text.ends_with('\n') {
        let _ = out.write_all(b"\n");
    }
    process::exit(0)
}
