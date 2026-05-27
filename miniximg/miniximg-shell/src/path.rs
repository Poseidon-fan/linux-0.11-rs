//! Path classification: image-side vs host-side.
//!
//! The shell speaks two filesystems at once: the loaded Minix image and
//! whatever the host process can see. To keep commands ergonomic, paths
//! default to the image; prefix with `@` to mean "this is a host path"
//! instead.
//!
//! Resolution rules:
//!
//! - `@/abs/path` → host absolute
//! - `@~/sub`     → host, with `~` expanded against `$HOME`
//! - `@./sub` / `@sub` → host, relative to the host cwd
//! - `/abs/path`  → image absolute
//! - `./sub` / `sub` / `../sub` → image, relative to the image cwd
//!
//! The two cwds live on [`Session`](crate::session::Session); this
//! module just classifies the raw string and produces a normalised form.

use std::path::{Path, PathBuf};

/// One parsed user-supplied path, tagged with which filesystem it lives in.
#[derive(Clone, Debug)]
pub enum AnyPath {
    /// Image-side path, always absolute after resolution.
    Image(String),
    /// Host-side path, absolute or relative-resolved against the host cwd.
    Host(PathBuf),
}

impl AnyPath {
    /// Returns `true` if this path refers to the host filesystem.
    pub fn is_host(&self) -> bool {
        matches!(self, AnyPath::Host(_))
    }
}

/// Resolves a raw user string against the active image / host cwds.
///
/// `host_cwd` is the host-side current directory used to anchor relative
/// host paths (e.g. `@./foo` or `@foo`). `image_cwd` is the absolute
/// image-side current directory used to anchor relative image paths.
pub fn resolve(raw: &str, image_cwd: &str, host_cwd: &Path) -> AnyPath {
    if let Some(rest) = raw.strip_prefix('@') {
        AnyPath::Host(resolve_host(rest, host_cwd))
    } else {
        AnyPath::Image(resolve_image(raw, image_cwd))
    }
}

/// Joins `raw` against `image_cwd` and normalises `.` / `..` components,
/// returning an absolute image-side path string. Symlinks aren't a
/// concept on this filesystem, so the result is purely syntactic.
pub fn resolve_image(raw: &str, image_cwd: &str) -> String {
    let combined = if raw.starts_with('/') {
        raw.to_string()
    } else if image_cwd.ends_with('/') {
        format!("{}{}", image_cwd, raw)
    } else {
        format!("{}/{}", image_cwd, raw)
    };
    normalise_image(&combined)
}

/// Walks the components of `path`, collapsing `.` / `..` and duplicate
/// slashes. Mirrors what `realpath -m` does, minus symlink resolution.
fn normalise_image(path: &str) -> String {
    let mut stack: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                stack.pop();
            }
            other => stack.push(other),
        }
    }
    if stack.is_empty() {
        "/".to_string()
    } else {
        let mut out = String::new();
        for part in stack {
            out.push('/');
            out.push_str(part);
        }
        out
    }
}

/// Expands a host path: `~` → `$HOME`, then resolves relative paths
/// against `host_cwd`. The returned path is **not** canonicalised — we
/// deliberately keep it lexical so the user sees what they typed.
fn resolve_host(raw: &str, host_cwd: &Path) -> PathBuf {
    let expanded = expand_tilde(raw);
    let candidate = PathBuf::from(expanded);
    if candidate.is_absolute() {
        candidate
    } else {
        host_cwd.join(candidate)
    }
}

fn expand_tilde(raw: &str) -> String {
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            let mut buf = PathBuf::from(home);
            buf.push(rest);
            return buf.to_string_lossy().into_owned();
        }
    } else if raw == "~"
        && let Some(home) = std::env::var_os("HOME")
    {
        return home.to_string_lossy().into_owned();
    }
    raw.to_string()
}

/// Returns the last component of an image path, or empty for the root.
pub fn image_basename(path: &str) -> &str {
    path.rsplit_once('/').map(|(_, name)| name).unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn image_relative_paths() {
        assert_eq!(resolve_image("foo", "/etc"), "/etc/foo");
        assert_eq!(resolve_image("./foo", "/etc"), "/etc/foo");
        assert_eq!(resolve_image("../bin/ls", "/etc"), "/bin/ls");
        assert_eq!(resolve_image("/abs", "/etc"), "/abs");
        assert_eq!(resolve_image("..", "/etc/x"), "/etc");
        assert_eq!(resolve_image("../..", "/etc/x"), "/");
    }

    #[test]
    fn host_prefix_marker() {
        let cwd = Path::new("/work");
        match resolve("@/tmp/x", "/", cwd) {
            AnyPath::Host(p) => assert_eq!(p, Path::new("/tmp/x")),
            _ => panic!("expected host"),
        }
        match resolve("@foo", "/", cwd) {
            AnyPath::Host(p) => assert_eq!(p, Path::new("/work/foo")),
            _ => panic!("expected host"),
        }
        match resolve("foo", "/etc", cwd) {
            AnyPath::Image(p) => assert_eq!(p, "/etc/foo"),
            _ => panic!("expected image"),
        }
    }

    #[test]
    fn parent_and_basename() {
        assert_eq!(image_basename("/etc/profile"), "profile");
        assert_eq!(image_basename("/"), "");
    }
}
