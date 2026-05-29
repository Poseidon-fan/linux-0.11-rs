//! End-to-end test runner for `linux-0.11-rs`.
//!
//! Boots the kernel under QEMU, drives the serial console with a small
//! scripting language (`.ktest` files), and reports pass/fail per test.
//!
//! Two selection modes:
//! * `--suite <name>` — run every `*.ktest` in a suite, resolved as
//!   `<suites-root>/<name>` (repeatable).
//! * `--test-set <suite.test>` — run a single test by `suite.test_name`
//!   (the `.ktest` is implicit; repeatable).
//!
//! With no selectors, every suite directory under the suites root is run.
//!
//! By default each test gets a fresh QEMU process **and** a fresh copy of
//! the disk image, so tests can mutate the disk freely without affecting
//! one another. `--disable-reboot` shares one QEMU (and one disk copy)
//! across the entire run — faster, but tests must clean up after
//! themselves.

mod runner;
mod script;
mod suite;

use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use clap::Parser;

use crate::{
    runner::{Runner, log_path_for},
    script::{Script, Step},
    suite::{TestCase, discover_default_suites, load_suite, load_test_set},
};

/// QEMU boot timeout — kernel + init + first prompt.
const BOOT_TIMEOUT: Duration = Duration::from_secs(30);
/// Default per-step timeout, overridable from a script via `! timeout N`.
const DEFAULT_STEP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Parser, Debug)]
#[command(name = "ktest", about = "End-to-end test runner for linux-0.11-rs")]
struct Cli {
    /// Kernel floppy image (e.g. kernel/Image-console-release).
    #[arg(long, value_name = "PATH")]
    kernel: PathBuf,

    /// Disk image to boot. A copy is taken per test by default.
    #[arg(long, value_name = "PATH")]
    image: PathBuf,

    /// Run every `.ktest` in this suite. Either a bare suite name
    /// (resolved under `--suites-root`) or a path to a directory.
    /// Repeatable.
    /// Run every `.ktest` in this suite (resolved under `--suites-root`).
    /// Repeatable.
    #[arg(long = "suite", value_name = "NAME")]
    suites: Vec<String>,

    /// Run one specific test, addressed as `suite.test_name`
    /// (no `.ktest` extension). Repeatable.
    #[arg(long = "test-set", value_name = "SUITE.TEST")]
    test_sets: Vec<String>,

    /// Share a single QEMU instance (and disk copy) for the whole run
    /// instead of rebooting between tests.
    #[arg(long)]
    disable_reboot: bool,

    /// Root directory containing the suites. Falls back to the
    /// `KTEST_SUITES_ROOT` environment variable; required if neither
    /// is set.
    #[arg(long, value_name = "DIR", env = "KTEST_SUITES_ROOT")]
    suites_root: PathBuf,

    /// Print captured serial output to stderr as soon as a test fails.
    #[arg(long)]
    show_log_on_fail: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("error: {:#}", e);
            ExitCode::from(2)
        }
    }
}

fn run(cli: &Cli) -> Result<bool> {
    for (label, p) in [("kernel", &cli.kernel), ("image", &cli.image)] {
        if !p.exists() {
            bail!("{} not found: {}", label, p.display());
        }
    }

    let tests = collect_tests(cli)?;
    if tests.is_empty() {
        bail!("no tests selected (try --suite or --test-set)");
    }

    fs::create_dir_all("target/test-out").ok();
    let tmpdir = mktempdir("ktest")?;

    let mut passed = 0usize;
    let mut failed: Vec<String> = Vec::new();

    if cli.disable_reboot {
        let disk = tmpdir.join("shared-disk.img");
        copy_image(&cli.image, &disk)?;
        let mut r = Runner::spawn(&cli.kernel, &disk)?;
        r.wait_boot(BOOT_TIMEOUT).context("kernel boot timed out")?;
        for test in &tests {
            run_one(&mut r, test, &mut passed, &mut failed, cli);
        }
    } else {
        // One fresh QEMU per test, with a private disk copy.
        for (i, test) in tests.iter().enumerate() {
            let disk = tmpdir.join(format!("disk-{}.img", i));
            if let Err(e) = copy_image(&cli.image, &disk) {
                failed.push(format!("{}: image copy failed: {:#}", test.label(), e));
                continue;
            }
            // Owned per-test runner — dropped at end of iteration kills QEMU.
            match Runner::spawn(&cli.kernel, &disk) {
                Ok(mut r) => {
                    if let Err(e) = r.wait_boot(BOOT_TIMEOUT) {
                        report_fail(test, &format!("boot: {:#}", e), Some(&r), cli);
                        failed.push(test.label());
                    } else {
                        run_one(&mut r, test, &mut passed, &mut failed, cli);
                    }
                }
                Err(e) => {
                    report_fail(test, &format!("spawn: {:#}", e), None, cli);
                    failed.push(test.label());
                }
            }
            let _ = fs::remove_file(&disk);
        }
    }

    let total = tests.len();
    println!();
    println!(
        "==> {} passed, {} failed, {} total",
        passed,
        failed.len(),
        total
    );
    for name in &failed {
        println!("    FAIL  {}", name);
    }

    // Best-effort cleanup; ignore errors so an open file (rare) doesn't
    // mask a real test failure.
    let _ = fs::remove_dir_all(&tmpdir);
    Ok(failed.is_empty())
}

fn collect_tests(cli: &Cli) -> Result<Vec<TestCase>> {
    let mut out = Vec::new();
    if cli.suites.is_empty() && cli.test_sets.is_empty() {
        let suites = discover_default_suites(&cli.suites_root)
            .with_context(|| format!("scanning {}", cli.suites_root.display()))?;
        for s in suites {
            out.extend(load_suite(&s)?);
        }
    }
    for spec in &cli.suites {
        out.extend(load_suite(&cli.suites_root.join(spec))?);
    }
    for spec in &cli.test_sets {
        out.push(load_test_set(&cli.suites_root, spec)?);
    }
    Ok(out)
}

fn run_one(
    runner: &mut Runner,
    test: &TestCase,
    passed: &mut usize,
    failed: &mut Vec<String>,
    cli: &Cli,
) {
    let label = test.label();
    print!("RUN   {} ... ", label);
    let started = Instant::now();
    let res = Script::load(&test.path, &label).and_then(|script| drive(runner, &script));
    let elapsed = started.elapsed();
    match res {
        Ok(()) => {
            println!("ok ({:?})", elapsed);
            *passed += 1;
        }
        Err(e) => {
            println!("FAIL ({:?})", elapsed);
            let msg = format!("{:#}", e);
            report_fail(test, &msg, Some(runner), cli);
            failed.push(label);
        }
    }
}

fn report_fail(test: &TestCase, msg: &str, runner: Option<&Runner>, cli: &Cli) {
    eprintln!("    {} :: {}", test.label(), msg);
    if let Some(r) = runner {
        let log_path = log_path_for(&test.label());
        if let Some(parent) = log_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let log = r.log_so_far();
        if let Err(e) = fs::write(&log_path, &log) {
            eprintln!("    (could not write log to {}: {})", log_path.display(), e);
        } else {
            eprintln!("    serial log: {}", log_path.display());
        }
        if cli.show_log_on_fail {
            eprintln!("    --- serial log ---");
            eprintln!("{}", String::from_utf8_lossy(&log));
            eprintln!("    --- end log ---");
        }
    }
}

fn drive(runner: &mut Runner, script: &Script) -> Result<()> {
    let mut step_timeout = DEFAULT_STEP_TIMEOUT;
    let mut last_chunk = String::new();
    // Assertion failures (Contains/Matches) accumulate so one run
    // surfaces every mismatch in the script instead of stopping at
    // the first. I/O-level errors (timeout, qemu died, bad regex)
    // still abort immediately.
    let mut failures: Vec<String> = Vec::new();
    for (i, step) in script.steps.iter().enumerate() {
        let line = script.line_numbers[i];
        let where_ = || format!("{}:{}", script.name, line);
        match step {
            Step::Send(cmd) => {
                last_chunk = runner
                    .send_line(cmd, step_timeout)
                    .with_context(|| format!("{}: send `{}`", where_(), cmd))?;
            }
            Step::ContainsLine(needle) => {
                if !last_chunk.contains(needle) {
                    failures.push(format!(
                        "{}: expected output to contain `{}`, got:\n{}",
                        where_(),
                        needle,
                        last_chunk
                    ));
                }
            }
            Step::MatchesRegex(pat) => {
                let re = regex::Regex::new(pat)
                    .with_context(|| format!("{}: bad regex `{}`", where_(), pat))?;
                if !re.is_match(&last_chunk) {
                    failures.push(format!(
                        "{}: expected output to match /{}/, got:\n{}",
                        where_(),
                        pat,
                        last_chunk
                    ));
                }
            }
            Step::SendRaw(payload) => {
                runner
                    .send_raw(payload)
                    .with_context(|| format!("{}: send-raw", where_()))?;
            }
            Step::WaitPrompt => {
                runner
                    .wait_prompt(step_timeout)
                    .with_context(|| format!("{}: wait-prompt", where_()))?;
                last_chunk.clear();
            }
            Step::ExpectSubstring(needle) => {
                runner
                    .expect_substring(needle, step_timeout)
                    .with_context(|| format!("{}: expect `{}`", where_(), needle))?;
            }
            Step::ExpectRegex(pat) => {
                runner
                    .expect_regex(pat, step_timeout)
                    .with_context(|| format!("{}: expect-regex /{}/", where_(), pat))?;
            }
            Step::Timeout(secs) => step_timeout = Duration::from_secs(*secs),
            Step::Sleep(ms) => std::thread::sleep(Duration::from_millis(*ms)),
        }
    }
    if !failures.is_empty() {
        let summary = format!(
            "{} assertion(s) failed:\n\n{}",
            failures.len(),
            failures.join("\n\n")
        );
        bail!(summary);
    }
    Ok(())
}

fn copy_image(src: &Path, dst: &Path) -> Result<()> {
    fs::copy(src, dst)
        .with_context(|| format!("copying disk image {} -> {}", src.display(), dst.display()))?;
    Ok(())
}

fn mktempdir(prefix: &str) -> Result<PathBuf> {
    let base = std::env::temp_dir();
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let path = base.join(format!("{}-{}-{:x}", prefix, pid, nanos));
    fs::create_dir_all(&path).with_context(|| format!("creating temp dir {}", path.display()))?;
    Ok(path)
}
