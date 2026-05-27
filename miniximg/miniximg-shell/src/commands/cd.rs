//! `cd` — change the image-side cwd.

use anyhow::Result;

use crate::{path, session::Session};

pub const NAME: &str = "cd";
pub const ALIASES: &[&str] = &[];
pub const SUMMARY: &str = "change the image-side directory";
pub const USAGE: &str = "cd [PATH]";

pub fn run(session: &mut Session, args: &[String]) -> Result<()> {
    let target = match args.first().map(String::as_str) {
        None | Some("") => "/".to_string(),
        Some(raw) => {
            let resolved = path::resolve(raw, session.image_cwd(), session.host_cwd());
            match resolved {
                path::AnyPath::Image(p) => p,
                path::AnyPath::Host(_) => {
                    anyhow::bail!("use `lcd` to change the host directory");
                }
            }
        }
    };
    session.set_image_cwd(target)
}
