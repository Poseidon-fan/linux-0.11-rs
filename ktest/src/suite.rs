//! Suite discovery — finds `.ktest` files under a suites root and
//! resolves `suite/test_name` identifiers used by `--test-set`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone)]
pub struct TestCase {
    /// Path to the `.ktest` file on disk.
    pub path: PathBuf,
    /// Suite name — the directory name containing the test.
    pub suite: String,
    /// Test name — the `.ktest` stem.
    pub name: String,
}

impl TestCase {
    /// Human-readable identifier (`sh/arith`) shown in CLI output and
    /// used as the log file basename.
    pub fn label(&self) -> String {
        format!("{}/{}", self.suite, self.name)
    }
}

/// Returns the list of suite directories directly under `root`.
pub fn discover_default_suites(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if !root.exists() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(root).with_context(|| format!("reading {}", root.display()))? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            out.push(entry.path());
        }
    }
    out.sort();
    Ok(out)
}

/// Loads every `*.ktest` in a suite directory, sorted by filename.
pub fn load_suite(dir: &Path) -> Result<Vec<TestCase>> {
    if !dir.is_dir() {
        bail!("suite is not a directory: {}", dir.display());
    }
    let suite = dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("ktest") {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();
            out.push(TestCase {
                path,
                suite: suite.clone(),
                name,
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Resolves a `suite.test_name` identifier against the suites root.
pub fn load_test_set(root: &Path, spec: &str) -> Result<TestCase> {
    let (suite, name) = spec
        .split_once('.')
        .with_context(|| format!("--test-set expects `suite.test`, got `{}`", spec))?;
    let path = root.join(suite).join(format!("{}.ktest", name));
    if !path.exists() {
        bail!("test not found: {}", path.display());
    }
    Ok(TestCase {
        path,
        suite: suite.to_string(),
        name: name.to_string(),
    })
}
