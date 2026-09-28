//! The `uncapped` mode: does `#![forbid(unsafe_code)]` hold in the default,
//! `derive` and `borrow` modes?
//!
//! Those modes compile every fixture and library with `--cap-lints allow`,
//! to keep warnings out of the first diagnostic. The cap applies to `forbid`
//! and `deny` as well, so no committed run of them enforced a single
//! `#![forbid(unsafe_code)]`, although every fixture and library declares
//! one. The `trace` mode runs uncapped and found this.
//!
//! Here each of the three modes runs twice, as a child process of this
//! binary: as committed, and with `--uncapped`, which drops the cap and
//! nothing else. A row whose result or first error differs between the two
//! is one the cap was hiding, and is printed with the uncapped diagnostic.
//! An uncapped run whose output differs at all is then printed whole, as the
//! corrected run. A control fixture, `unsafe` under `forbid`, shows the flag
//! does what it says: it builds capped and fails uncapped.

use std::path::Path;
use std::process::Command;

use super::{first_diagnostic, rustc, version};

/// (the mode's argument, its name in the report).
const MODES: [(&str, &str); 3] = [("", "default"), ("derive", "derive"), ("borrow", "borrow")];

/// The control, `uncapped/control.rs`: whether it builds with the cap and
/// without, and the uncapped first error.
fn control(root: &Path) -> (bool, bool, String) {
    let out = root.join("target/rfd-0003-brand-erasure/uncapped");
    std::fs::create_dir_all(&out).expect("the output directory");
    let build = |cap: &[&str]| {
        let output = rustc(root, None)
            .args([
                "--edition",
                "2024",
                "--crate-type",
                "bin",
                "--emit=metadata",
            ])
            .args(cap)
            .args(["--color", "never", "-o"])
            .arg(out.join("control.rmeta"))
            .arg("fixtures/rfd-0003-brand-erasure/uncapped/control.rs")
            .output()
            .expect("rustc runs");
        let first = first_diagnostic(&String::from_utf8_lossy(&output.stderr));
        (
            output.status.success(),
            first.first().cloned().unwrap_or_default(),
        )
    };
    let (capped, _) = build(&["--cap-lints", "allow"]);
    let (uncapped, first) = build(&[]);
    (capped, uncapped, first)
}

/// One mode's run: stdout, and stderr if it failed.
struct Run {
    stdout: String,
    failed: Option<String>,
}

fn run_mode(root: &Path, mode: &str, uncapped: bool) -> Run {
    // Cargo may relink this binary while it runs (another probe's build
    // shares the target directory), and Linux then names the old one
    // `... (deleted)`; the new one is at the path without the suffix.
    let exe = std::env::current_exe().expect("this binary's path");
    let exe = exe.to_string_lossy();
    let mut command = Command::new(exe.trim_end_matches(" (deleted)"));
    command.current_dir(root);
    if !mode.is_empty() {
        command.arg(mode);
    }
    if uncapped {
        command.arg("--uncapped");
    }
    let output = command.output().expect("the mode runs");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        failed: (!output.status.success())
            .then(|| String::from_utf8_lossy(&output.stderr).into_owned()),
    }
}

/// A table row: the words before the expectation (design and fixture, or
/// library and fixture), and the result with the first error.
struct Row {
    key: String,
    result: String,
}

/// The table rows of a mode's output: a line with an expectation (`build`,
/// `fail`, `fail?`) followed by a result (`built`, `failed`).
fn rows(stdout: &str) -> Vec<Row> {
    let mut rows = Vec::new();
    for line in stdout.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        let Some(i) = words.windows(2).position(|w| {
            matches!(w[0], "build" | "fail" | "fail?") && matches!(w[1], "built" | "failed")
        }) else {
            continue;
        };
        rows.push(Row {
            key: words[..i].join(" "),
            result: words[i + 1..].join(" "),
        });
    }
    rows
}

/// The `--- ..., first diagnostic` section for a row's fixture, if the mode
/// printed one.
fn diagnostic<'a>(stdout: &'a str, key: &str) -> Option<Vec<&'a str>> {
    // The sections name the fixture as `design: fixture.rs` in the default
    // and `borrow` modes, and `library: fixture` in the `derive` mode.
    let mut words = key.splitn(2, ' ');
    let (first, rest) = (words.next()?, words.next()?);
    let heads = [
        format!("--- {first}: {rest}.rs, first diagnostic"),
        format!("--- {first}: {rest}, first diagnostic"),
    ];
    let mut lines = stdout.lines().skip_while(|l| !heads.iter().any(|h| l == h));
    let head = lines.next()?;
    let mut section = vec![head];
    section.extend(lines.take_while(|l| !l.is_empty()));
    Some(section)
}

pub fn main(root: &Path) {
    println!("stable:  {}  (rust-toolchain.toml)", version(root, None));
    println!("each mode run twice: as committed (--cap-lints allow), and with --uncapped");
    let (capped, uncapped, first) = control(root);
    let built = |b: bool| if b { "built" } else { "failed" };
    println!(
        "control: uncapped/control.rs, unsafe under #![forbid(unsafe_code)]: capped {}, uncapped {} ({first})",
        built(capped),
        built(uncapped)
    );
    println!();

    let mut uncapped_runs = Vec::new();
    let mut changed_total = 0;
    for (mode, name) in MODES {
        let capped = run_mode(root, mode, false);
        let uncapped = run_mode(root, mode, true);
        for (label, run) in [("capped", &capped), ("uncapped", &uncapped)] {
            if let Some(stderr) = &run.failed {
                println!("{name}, {label}: the mode itself failed:");
                for line in stderr.lines().take(40) {
                    println!("  {line}");
                }
            }
        }
        let (before, after) = (rows(&capped.stdout), rows(&uncapped.stdout));
        assert_eq!(
            before.iter().map(|r| &r.key).collect::<Vec<_>>(),
            after.iter().map(|r| &r.key).collect::<Vec<_>>(),
            "{name}: both runs have the same rows"
        );
        let changed: Vec<(&Row, &Row)> = before
            .iter()
            .zip(&after)
            .filter(|(b, a)| b.result != a.result)
            .collect();
        println!(
            "{name}: {} rows, {} changed without the cap; whole output {}",
            before.len(),
            changed.len(),
            if capped.stdout == uncapped.stdout {
                "byte-identical"
            } else {
                "differs"
            }
        );
        for (b, a) in &changed {
            println!("  {}", b.key);
            println!("    capped:   {}", b.result);
            println!("    uncapped: {}", a.result);
        }
        for (_, a) in &changed {
            if let Some(section) = diagnostic(&uncapped.stdout, &a.key) {
                println!();
                for line in section {
                    println!("  {line}");
                }
            }
        }
        println!();
        changed_total += changed.len();
        // A run is printed whole only if it differs: then it is the corrected
        // run, and the committed one was wrong.
        if capped.stdout != uncapped.stdout {
            uncapped_runs.push((mode, uncapped.stdout));
        }
    }

    for (mode, stdout) in &uncapped_runs {
        let args = if mode.is_empty() {
            "--uncapped".to_string()
        } else {
            format!("{mode} --uncapped")
        };
        println!("=== cargo run --release --bin rfd-0003-brand-erasure -- {args}");
        print!("{stdout}");
        println!();
    }
    println!(
        "uncapped: {changed_total} rows of the default, derive and borrow modes changed when every #![forbid(unsafe_code)] was enforced"
    );
}
