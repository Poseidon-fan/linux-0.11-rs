#![no_std]
#![no_main]

use core::{ffi::CStr, str};

use user_lib::{env, println};

#[user_lib::main]
fn main() -> i32 {
    println!("runtime smoke test");

    match user_lib::syscall::misc::test() {
        Ok(value) => println!("test syscall returned {}", value),
        Err(errno) => println!("test syscall failed with errno {}", errno.code()),
    }

    let args = env::args();
    println!("argc = {}", args.len());
    for (index, arg) in args.iter().enumerate() {
        print_cstr("argv", index, arg);
    }

    let mut env_count = 0;
    for (index, entry) in env::vars().iter().enumerate() {
        env_count += 1;
        print_cstr("envp", index, entry);
    }
    println!("envc = {}", env_count);

    match env::var(b"HOME") {
        Some(home) => print_bytes("HOME", home),
        None => println!("HOME is not set"),
    }

    0
}

fn print_cstr(prefix: &str, index: usize, value: &CStr) {
    match str::from_utf8(value.to_bytes()) {
        Ok(text) => println!("{}[{}] = {}", prefix, index, text),
        Err(_) => println!(
            "{}[{}] = <{} non-UTF-8 bytes>",
            prefix,
            index,
            value.to_bytes().len()
        ),
    }
}

fn print_bytes(name: &str, bytes: &[u8]) {
    match str::from_utf8(bytes) {
        Ok(text) => println!("{} = {}", name, text),
        Err(_) => println!("{} = <{} non-UTF-8 bytes>", name, bytes.len()),
    }
}
