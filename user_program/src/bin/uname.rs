//! `uname` — print system information.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::String, vec::Vec};
use core::str;

use user_lib::{
    eprintln,
    io::{self, Write},
    process::ExitCode,
    syscall::{self, process::UtsName},
};
use user_program::cli::cli_args;

cli_args! {
    /// Print certain system information.
    pub struct UnameArgs {
        /// Print all information, in the standard field order.
        pub all:      bool = ["-a", "--all"],
        /// Print the kernel name.
        pub kernel:   bool = ["-s", "--kernel-name"],
        /// Print the network node hostname.
        pub nodename: bool = ["-n", "--nodename"],
        /// Print the kernel release.
        pub release:  bool = ["-r", "--kernel-release"],
        /// Print the kernel version.
        pub version:  bool = ["-v", "--kernel-version"],
        /// Print the machine hardware name.
        pub machine:  bool = ["-m", "--machine"],
        /// Print the processor type, or unknown if unavailable.
        pub processor: bool = ["-p", "--processor"],
        /// Print the hardware platform, or unknown if unavailable.
        pub platform: bool = ["-i", "--hardware-platform"],
        /// Print the operating system.
        pub operating_system: bool = ["-o", "--operating-system"],
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let args = UnameArgs::parse_env_or_exit();
    let mut uts = empty_uts_name();
    if let Err(err) = syscall::process::uname(&mut uts as *mut UtsName) {
        eprintln!("uname: errno {}", err.code());
        return ExitCode::FAILURE;
    }

    let fields = selected_fields(&args, &uts);
    let mut out = io::stdout();
    for (index, field) in fields.iter().enumerate() {
        if index > 0 && out.write_all(b" ").is_err() {
            return ExitCode::FAILURE;
        }
        if out.write_all(field.as_bytes()).is_err() {
            return ExitCode::FAILURE;
        }
    }
    if out.write_all(b"\n").is_err() {
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

/// Builds the selected uname fields in POSIX/GNU display order.
fn selected_fields(args: &UnameArgs, uts: &UtsName) -> Vec<String> {
    let show_default = !args.all
        && !args.kernel
        && !args.nodename
        && !args.release
        && !args.version
        && !args.machine
        && !args.processor
        && !args.platform
        && !args.operating_system;
    let mut fields = Vec::new();
    if args.all || args.kernel || show_default {
        fields.push(uts_field(&uts.sysname));
    }
    if args.all || args.nodename {
        fields.push(uts_field(&uts.nodename));
    }
    if args.all || args.release {
        fields.push(uts_field(&uts.release));
    }
    if args.all || args.version {
        fields.push(uts_field(&uts.version));
    }
    if args.all || args.machine {
        fields.push(uts_field(&uts.machine));
    }
    if args.processor {
        fields.push(String::from("unknown"));
    }
    if args.platform {
        fields.push(String::from("unknown"));
    }
    if args.all || args.operating_system {
        fields.push(uts_field(&uts.sysname));
    }
    fields
}

/// Converts one fixed-width utsname field into an owned UTF-8 string.
fn uts_field(bytes: &[u8]) -> String {
    let len = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    str::from_utf8(&bytes[..len]).unwrap_or("").into()
}

/// Creates a zeroed utsname buffer for the kernel to fill.
fn empty_uts_name() -> UtsName {
    UtsName {
        sysname: [0; 9],
        nodename: [0; 9],
        release: [0; 9],
        version: [0; 9],
        machine: [0; 9],
    }
}
