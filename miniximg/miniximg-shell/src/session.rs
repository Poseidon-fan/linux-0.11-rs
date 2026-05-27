//! Per-shell state: the loaded image, both cwds, and config knobs.

use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};
use miniximg::{InodeType, MinixFileSystem, NodeMetadata};

/// Configuration passed to [`crate::run`].
pub struct ShellOptions {
    /// Path to the Minix v1 filesystem image to operate on.
    pub image: PathBuf,
    /// When true, every mutating command is refused with a friendly error.
    pub readonly: bool,
    /// Where to read / write line-edit history.
    ///
    /// `None` keeps history in memory only; `Some(path)` persists across
    /// sessions, creating the file (and any missing parents) on save.
    pub history: Option<PathBuf>,
}

/// The image plus the two working directories, plus the bits commands
/// need to render diagnostics consistently.
pub struct Session {
    /// Loaded image filesystem.
    fs: MinixFileSystem<File>,
    /// Source path of the image, retained for the prompt and for error
    /// messages.
    image_path: PathBuf,
    /// Image-side current directory, always absolute and normalised.
    image_cwd: String,
    /// Host-side current directory, used to anchor `@`-prefixed relative
    /// paths and `lcd` / `lls`.
    host_cwd: PathBuf,
    /// Refuse modifications when set.
    pub readonly: bool,
    /// Pre-resolved history path passed through to [`crate::editor`].
    pub history: Option<PathBuf>,
}

impl Session {
    /// Opens the image and prepares a session.
    pub fn open(options: ShellOptions) -> Result<Self> {
        let image_path = options.image.clone();
        let file = OpenOptions::new()
            .read(true)
            .write(!options.readonly)
            .open(&image_path)
            .with_context(|| format!("failed to open image {}", image_path.display()))?;
        let fs = MinixFileSystem::open(file).with_context(|| {
            format!(
                "{} is not a valid Minix v1 filesystem image",
                image_path.display()
            )
        })?;
        let host_cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));

        Ok(Self {
            fs,
            image_path,
            image_cwd: "/".to_string(),
            host_cwd,
            readonly: options.readonly,
            history: options.history,
        })
    }

    /// Image filename used in the prompt — basename only so a long
    /// absolute path doesn't dominate the line.
    pub fn image_label(&self) -> String {
        self.image_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.image_path.display().to_string())
    }

    /// Returns the image-side working directory.
    pub fn image_cwd(&self) -> &str {
        &self.image_cwd
    }

    /// Returns the host-side working directory.
    pub fn host_cwd(&self) -> &Path {
        &self.host_cwd
    }

    /// Borrow the underlying filesystem mutably (every operation on the
    /// core lib takes `&mut self`).
    pub fn fs_mut(&mut self) -> &mut MinixFileSystem<File> {
        &mut self.fs
    }

    /// Sets the image-side cwd after validating it exists and is a
    /// directory.
    pub fn set_image_cwd(&mut self, new: String) -> Result<()> {
        let meta = self.fs.stat(&new).with_context(|| format!("cd: {}", new))?;
        if meta.kind != InodeType::Directory {
            return Err(anyhow!("cd: {}: not a directory", new));
        }
        self.image_cwd = new;
        Ok(())
    }

    /// Sets the host-side cwd after validating it exists and is a directory.
    pub fn set_host_cwd(&mut self, new: PathBuf) -> Result<()> {
        let meta = std::fs::metadata(&new).with_context(|| format!("lcd: {}", new.display()))?;
        if !meta.is_dir() {
            return Err(anyhow!("lcd: {}: not a directory", new.display()));
        }
        self.host_cwd = new;
        Ok(())
    }

    /// Helper used by mutating commands: returns `Err` in readonly mode
    /// with a clear message naming the offending verb.
    pub fn require_writable(&self, verb: &str) -> Result<()> {
        if self.readonly {
            Err(anyhow!("{}: image is open read-only", verb))
        } else {
            Ok(())
        }
    }

    /// Convenience: stat an image path through the active filesystem.
    pub fn stat_image(&mut self, path: &str) -> Result<NodeMetadata> {
        self.fs.stat(path).map_err(Into::into)
    }
}
