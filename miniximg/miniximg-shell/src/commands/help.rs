//! `help` — print command list or per-command usage.

use anyhow::Result;

use crate::{commands, session::Session};

pub const NAME: &str = "help";
pub const ALIASES: &[&str] = &["?"];
pub const SUMMARY: &str = "list commands or describe one";
pub const USAGE: &str = "help [COMMAND]";

pub fn run(_session: &mut Session, args: &[String]) -> Result<()> {
    if let Some(name) = args.first() {
        match commands::lookup(name) {
            Some(cmd) => {
                println!("{} — {}", cmd.name, cmd.summary);
                println!("usage: {}", cmd.usage);
                if !cmd.aliases.is_empty() {
                    println!("aliases: {}", cmd.aliases.join(", "));
                }
            }
            None => anyhow::bail!("no such command: {}", name),
        }
        return Ok(());
    }

    println!("Image-side commands:");
    print_table(&[
        "ls", "cd", "cat", "stat", "tree", "mkdir", "rmdir", "rm", "mv", "ln", "touch", "mknod",
        "chmod", "edit",
    ]);
    println!();
    println!("Host-side commands:");
    print_table(&["lls", "lcd", "lpwd"]);
    println!();
    println!("Cross-filesystem:");
    print_table(&["put", "get", "cp", "diff"]);
    println!();
    println!("Meta:");
    print_table(&["pwd", "info", "fsck", "sync", "clear", "help", "exit"]);
    println!();
    println!("Prefix any path with `@` to treat it as a host path.");
    println!("Run `help COMMAND` for details on one command.");
    Ok(())
}

fn print_table(names: &[&str]) {
    for name in names {
        if let Some(cmd) = commands::lookup(name) {
            println!("  {:<8} {}", cmd.name, cmd.summary);
        }
    }
}
