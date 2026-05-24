//! Minimal user-space runtime support.
//!
//! This module is enabled by the `runtime` feature. It owns the process entry
//! contract above the raw syscall layer: the `_start` shim decodes the initial
//! stack, builds lightweight argument/environment views, runs `main`, and exits
//! with its return status.

use core::panic::PanicInfo;

use crate::println;

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
/// words and then calls [`run`]. The provided function must have this
/// signature:
///
/// ```rust,ignore
/// fn main() -> i32
/// ```
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

/// User program entry function accepted by [`crate::main`].
pub type Main = fn() -> i32;

/// Runs the user program and exits with the returned status code.
///
/// # Safety
///
/// `argv` must point to an array containing at least `argc` valid
/// NUL-terminated string pointers followed by a NULL terminator. `envp` must
/// point to a NULL-terminated array of valid NUL-terminated string pointers, or
/// be NULL. These pointers must remain valid for the duration of `main`.
#[inline]
pub unsafe fn run(main: Main, argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    unsafe {
        crate::env::init(argc, argv, envp);
    }
    let status = main();
    crate::exit(status as u32)
}

/// Prints panic information and terminates the process with a conventional
/// failure status.
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("panic: {}", info);
    crate::exit(101)
}
