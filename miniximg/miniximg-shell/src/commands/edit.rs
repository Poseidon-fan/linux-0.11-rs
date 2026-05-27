//! `edit` — open an image-side file in `$EDITOR`, write back on save.
//!
//! Workflow:
//! 1. Extract the image file's bytes into a host temp file.
//! 2. Spawn `$EDITOR` (or `$VISUAL`, then `vi` as a fallback).
//! 3. If the editor exits successfully **and** the temp file's contents
//!    differ from the original, write the new bytes back into the image.
//! 4. Unlink the temp file unconditionally.

use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow};
use miniximg::MinixError;

use crate::{path as shellpath, session::Session};

pub const NAME: &str = "edit";
pub const ALIASES: &[&str] = &[];
pub const SUMMARY: &str = "open an image file in $EDITOR";
pub const USAGE: &str = "edit PATH";

pub fn run(session: &mut Session, args: &[String]) -> Result<()> {
    session.require_writable("edit")?;
    super::expect_args(args, 1, Some(1))?;

    let cwd = session.image_cwd().to_string();
    let img_path = shellpath::resolve_image(&args[0], &cwd);

    // Missing files are fine — the editor just opens an empty buffer.
    let existing = match session.fs_mut().read_file_at_path(&img_path) {
        Ok(bytes) => bytes,
        Err(MinixError::NotFound { .. }) => Vec::new(),
        Err(e) => return Err(e.into()),
    };

    let tmp = make_tempfile(&img_path)?;
    let _guard = TempFileGuard(tmp.clone());

    OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&tmp)
        .with_context(|| format!("opening {}", tmp.display()))?
        .write_all(&existing)?;

    run_editor(&tmp)?;

    // If the editor exited but didn't modify the buffer, no-op.
    let mut new_bytes = Vec::new();
    OpenOptions::new()
        .read(true)
        .open(&tmp)?
        .read_to_end(&mut new_bytes)?;
    if new_bytes == existing {
        println!("[unchanged]");
        return Ok(());
    }

    session.fs_mut().write_file_at_path(
        &img_path,
        &new_bytes,
        &super::default_node_options(0o644),
        true,
        &super::default_parent_options(),
    )?;
    session.fs_mut().flush()?;
    println!("[saved {} bytes]", new_bytes.len());
    Ok(())
}

/// RAII helper that unlinks the host temp file on drop, regardless of
/// whether the editor or the write-back succeeded.
struct TempFileGuard(PathBuf);

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn run_editor(file: &Path) -> Result<()> {
    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "vi".to_string());
    let status = Command::new(&editor)
        .arg(file)
        .status()
        .with_context(|| format!("failed to spawn editor `{}`", editor))?;
    if !status.success() {
        return Err(anyhow!("editor exited with {}", status));
    }
    Ok(())
}

/// Builds a unique temp file path under `$TMPDIR`. The name embeds the
/// pid, a nanosecond timestamp, and the basename being edited so that
/// concurrent sessions and lingering files don't collide.
fn make_tempfile(image_path: &str) -> Result<PathBuf> {
    let stem = shellpath::image_basename(image_path);
    let stem = if stem.is_empty() { "edit" } else { stem };
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let name = format!("miniximg-edit-{}-{}-{}", std::process::id(), nanos, stem);
    let path = std::env::temp_dir().join(name);
    std::fs::File::create(&path)
        .with_context(|| format!("creating temp file {}", path.display()))?;
    Ok(path)
}
