//! User-space runtime services.
//!
//! Counterpart to [`std::rt`]: this module is the bridge between the
//! kernel-provided process entry point and the user program's `main`
//! function. It owns the assembly `_start` shim, decodes the initial
//! argument and environment vectors off the stack, dispatches into `main`,
//! and routes its return value through [`crate::process::Termination`] so
//! the resulting [`crate::process::ExitCode`] terminates the process.
//!
//! What a program's exit *means* lives in [`crate::process`]; this module
//! only orchestrates the entry call and the handoff back to the kernel.

use core::panic::PanicInfo;

use crate::{println, process};

/// Defines the user-space process entry point.
///
/// The kernel enters a freshly executed program with this stack layout:
///
/// ```text
/// +------------------+ Low address, current ESP
/// |       argc       |
/// |       argv       | --> argv[0], argv[1], ..., NULL
/// |       envp       | --> envp[0], envp[1], ..., NULL
/// +------------------+
/// | pointer tables   |
/// +------------------+
/// | argument strings |
/// | environment      |
/// +------------------+ High address
/// ```
///
/// This macro emits a small assembly `_start` shim that reads those stack
/// words and then calls [`run`]. The provided function may have any return
/// type that implements [`crate::process::Termination`], including `()`,
/// [`crate::process::ExitCode`], and `Result<T, E>`.
#[doc(hidden)]
#[macro_export]
macro_rules! __user_lib_entry {
    ($main:path) => {
        core::arch::global_asm!(
            r#"
            .section .text._start,"ax"
            .globl _start
            .type _start, @function
        _start:
            movl (%esp), %eax
            movl 4(%esp), %ebx
            movl 8(%esp), %ecx
            pushl %ecx
            pushl %ebx
            pushl %eax
            call __user_lib_start
        1:
            jmp 1b
            .size _start, . - _start
            "#,
            options(att_syntax),
        );

        #[doc(hidden)]
        #[unsafe(no_mangle)]
        extern "C" fn __user_lib_start(
            argc: usize,
            argv: *const *const u8,
            envp: *const *const u8,
        ) -> ! {
            unsafe { $crate::rt::run($main, argc, argv, envp) }
        }
    };
}

/// Runs the user program and exits with the [`process::ExitCode`] produced by
/// its return value.
///
/// # Safety
///
/// `argv` must point to an array containing at least `argc` valid
/// NUL-terminated string pointers followed by a NULL terminator. `envp` must
/// point to a NULL-terminated array of valid NUL-terminated string pointers, or
/// be NULL. These pointers must remain valid for the duration of `main`.
#[inline]
pub unsafe fn run<T: process::Termination>(
    main: fn() -> T,
    argc: usize,
    argv: *const *const u8,
    envp: *const *const u8,
) -> ! {
    unsafe {
        crate::env::init(argc, argv, envp);
    }
    main().report().exit_process()
}

/// Prints panic information and terminates the process with a conventional
/// failure status.
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("panic: {}", info);
    process::exit(101)
}
