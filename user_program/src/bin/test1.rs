#![no_std]
#![no_main]

use user_lib::{env, println};

#[user_lib::main]
fn main() {
    println!("runtime smoke test");

    match user_lib::syscall::misc::test() {
        Ok(value) => println!("test syscall returned {}", value),
        Err(errno) => println!("test syscall failed with errno {}", errno.code()),
    }

    let mut argc = 0;
    for (index, arg) in env::args().enumerate() {
        argc += 1;
        println!("argv[{}] = {}", index, arg);
    }
    println!("argc = {}", argc);

    let mut env_count = 0;
    for (index, (name, value)) in env::vars().enumerate() {
        env_count += 1;
        println!("envp[{}] = {}={}", index, name, value);
    }
    println!("envc = {}", env_count);

    match env::var("HOME") {
        Ok(home) => println!("HOME = {}", home),
        Err(error) => println!("HOME lookup failed: {:?}", error),
    };
}
