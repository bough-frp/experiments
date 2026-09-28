//! The `derive` mode: can a real `#[derive(Rebrand)]` write the impls the
//! main run's fixtures wrote by hand, with a copy-free `view` for a type
//! with no brand, and what do the orphan rules force for another crate's
//! types?
//!
//! The derive is the workspace member `rfd-0003-rebrand-derive`. The
//! fixtures under `fixtures/rfd-0003-brand-erasure/derive/` are compiled as
//! a user's crate against three others, so the orphan rules apply as they
//! would for real:
//!
//! - `bough`, from `derive/bough.rs`: the main run's `api.rs`, unchanged,
//!   as a crate of its own, with the derive re-exported next to its trait
//!   and `Rebrand` for `Box` and `HashMap`.
//! - `foreign`, from `derive/foreign.rs`: a third crate's type.
//! - `bough` again, from `derive/blanket.rs`, for the `blanket` rows: a
//!   Bough with `impl<T: Clone + 'static> Rebrand for T`, types only.
//!
//! The proc macro is built by Cargo, which knows how to build `syn`, into a
//! target directory of its own so the call never waits on the lock of the
//! `cargo run` that started this binary. Everything else is built by the
//! pinned `rustc`, as in the main run, with `--extern` naming each crate:
//! a generated Cargo project per fixture would bury the first diagnostic
//! in Cargo's output and rebuild the libraries for every fixture.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::{Mode, Outcome, cap_lints, first_diagnostic, run, rustc, version};
use Expect::{Build, Fail};
use Mode::{Check, Run};

const DIR: &str = "fixtures/rfd-0003-brand-erasure/derive";

#[derive(Clone, Copy, PartialEq)]
enum Expect {
    Build,
    Fail,
}

/// (the Bough it builds against, fixture, expected, whether it runs). A
/// fixture of `-` is the Bough library itself, built with `--cfg
/// with_token`.
const CASES: &[(&str, &str, Expect, Mode)] = &[
    // The derive on the api's own runtime.
    ("bough", "panels", Build, Run),
    ("bough", "choice", Build, Run),
    ("bough", "tagged", Build, Run),
    ("bough", "view", Build, Run),
    ("bough", "leaf", Build, Run),
    ("bough", "bounded", Build, Check),
    ("bough", "missing-impl", Fail, Check),
    ("bough", "foreign-field", Fail, Check),
    ("bough", "foreign-stream", Fail, Check),
    ("bough", "skip-token", Fail, Check),
    ("bough", "skip-param", Fail, Check),
    ("bough", "brand-free-token", Fail, Check),
    ("bough", "two-lifetimes", Fail, Check),
    // The orphan rules, and a blanket impl as the way around them.
    ("bough", "orphan-impl", Fail, Check),
    ("blanket", "blanket-plain", Build, Check),
    ("blanket", "blanket-clone", Fail, Check),
    ("blanket", "blanket-token", Fail, Check),
    ("blanket", "-", Fail, Check),
];

struct Libraries {
    out: PathBuf,
    /// The directory the proc macro was built in, for `-L`.
    deps: PathBuf,
    derive: PathBuf,
    bough: PathBuf,
    blanket: PathBuf,
    foreign: PathBuf,
}

/// Build the proc macro with Cargo, into `target/rfd-0003-brand-erasure/`
/// `dir` with `features`, and return the path of its `.so`.
pub(super) fn build_derive(root: &Path, dir: &str, features: &[&str]) -> PathBuf {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let target = root.join("target/rfd-0003-brand-erasure").join(dir);
    let mut command = Command::new(cargo);
    command
        .current_dir(root)
        .args(["build", "--release", "-p", "rfd-0003-rebrand-derive"]);
    if !features.is_empty() {
        command.arg("--features").arg(features.join(","));
    }
    let output = command
        .args(["--message-format=json", "--target-dir"])
        .arg(&target)
        .output()
        .expect("cargo runs");
    assert!(
        output.status.success(),
        "the derive builds:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Cargo's JSON names the artifact; a string search saves a JSON parser.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .find(|l| l.contains("\"compiler-artifact\"") && l.contains("\"rfd_0003_rebrand_derive\""))
        .expect("cargo reports the derive");
    let start = line.find("\"filenames\":[\"").expect("its filenames") + 14;
    let end = start + line[start..].find('"').expect("a filename");
    PathBuf::from(&line[start..end])
}

/// Build one library with the pinned `rustc`, returning the outcome.
fn library(
    root: &Path,
    libs: &Libraries,
    source: &str,
    name: &str,
    cfg: Option<&str>,
) -> (PathBuf, Outcome) {
    let dir = libs.out.join(source.trim_end_matches(".rs"));
    std::fs::create_dir_all(&dir).expect("the library directory");
    let rlib = dir.join(format!("lib{name}.rlib"));
    let mut command = rustc(root, None);
    command.args([
        "--edition",
        "2024",
        "--crate-type",
        "rlib",
        "--crate-name",
        name,
    ]);
    if let Some(cfg) = cfg {
        command.args(["--cfg", cfg]);
    }
    let output = command
        .arg("--extern")
        .arg(format!("rfd_0003_rebrand_derive={}", libs.derive.display()))
        .args(cap_lints())
        .args(["--color", "never", "-o"])
        .arg(&rlib)
        .arg(Path::new(DIR).join(source))
        .output()
        .expect("rustc runs");
    let outcome = Outcome {
        built: output.status.success(),
        first: first_diagnostic(&String::from_utf8_lossy(&output.stderr)),
        ran: None,
    };
    (rlib, outcome)
}

fn fixture(root: &Path, libs: &Libraries, bough: &str, fixture: &str, mode: Mode) -> Outcome {
    let binary = libs.out.join(format!("{bough}-{fixture}"));
    let lib = if bough == "blanket" {
        &libs.blanket
    } else {
        &libs.bough
    };
    let mut command = rustc(root, None);
    command
        .args(["--edition", "2024", "--crate-type", "bin", "--crate-name"])
        .arg(fixture.replace('-', "_"))
        .arg("--extern")
        .arg(format!("bough={}", lib.display()))
        .arg("--extern")
        .arg(format!("foreign={}", libs.foreign.display()))
        .arg("-L")
        .arg(format!("dependency={}", libs.deps.display()));
    match mode {
        Check => command
            .args(["--emit=metadata", "-o"])
            .arg(binary.with_extension("rmeta")),
        Run => command.arg("-o").arg(&binary),
    };
    let output = command
        .args(cap_lints())
        .args(["--color", "never"])
        .arg(Path::new(DIR).join(format!("{fixture}.rs")))
        .output()
        .expect("rustc runs");
    let built = output.status.success();
    Outcome {
        built,
        first: first_diagnostic(&String::from_utf8_lossy(&output.stderr)),
        ran: (built && mode == Run).then(|| run(&binary)),
    }
}

pub fn main(root: &Path) {
    println!("stable:  {}  (rust-toolchain.toml)", version(root, None));
    let derive = build_derive(root, "derive-build", &[]);
    let out = root.join("target/rfd-0003-brand-erasure/derive");
    std::fs::create_dir_all(&out).expect("the output directory");
    let mut libs = Libraries {
        deps: derive.parent().expect("the deps directory").to_path_buf(),
        derive,
        bough: PathBuf::new(),
        blanket: PathBuf::new(),
        foreign: PathBuf::new(),
        out,
    };
    for (source, name) in [
        ("foreign.rs", "foreign"),
        ("bough.rs", "bough"),
        ("blanket.rs", "bough"),
    ] {
        let (rlib, outcome) = library(root, &libs, source, name, None);
        assert!(outcome.built, "{source} builds: {:?}", outcome.first);
        match source {
            "foreign.rs" => libs.foreign = rlib,
            "bough.rs" => libs.bough = rlib,
            _ => libs.blanket = rlib,
        }
    }
    println!(
        "derive:  rfd-0003-rebrand-derive, built by cargo; libraries: bough, foreign, blanket"
    );
    println!();
    println!(
        "{:<8} {:<17} {:<8} {:<7} first error",
        "bough", "fixture", "expected", "result"
    );

    let (mut builds, mut built, mut fails, mut failed) = (0, 0, 0, 0);
    let mut outcomes = Vec::new();
    for &(bough, name, expect, mode) in CASES {
        let (label, outcome) = if name == "-" {
            // Written to a directory of its own, so the blanket library the
            // fixtures use is left as it was.
            let (_, outcome) = library(
                root,
                &libs,
                "blanket.rs",
                "bough_with_token",
                Some("with_token"),
            );
            ("--cfg with_token", outcome)
        } else {
            (name, fixture(root, &libs, bough, name, mode))
        };
        match expect {
            Build => {
                builds += 1;
                built += usize::from(outcome.built);
            }
            Fail => {
                fails += 1;
                failed += usize::from(!outcome.built);
            }
        }
        println!(
            "{:<8} {:<17} {:<8} {:<7} {}",
            bough,
            label,
            if expect == Build { "build" } else { "fail" },
            if outcome.built { "built" } else { "failed" },
            outcome.headline(),
        );
        outcomes.push((bough, label, outcome));
    }
    println!();

    for (bough, name, outcome) in &outcomes {
        if let Some(lines) = &outcome.ran {
            println!("--- {bough}: {name}.rs, run");
            for line in lines {
                println!("{line}");
            }
            println!();
        }
    }
    // Every failure's whole first diagnostic: the text a user would see.
    for (bough, name, outcome) in &outcomes {
        if !outcome.built {
            println!("--- {bough}: {name}, first diagnostic");
            for line in &outcome.first {
                println!("{line}");
            }
            println!();
        }
    }
    println!(
        "derive: {built} of {builds} must-build programs built, {failed} of {fails} must-fail programs failed"
    );
    println!(
        "(every fixture and library is #![forbid(unsafe_code)]; no Rebrand impl in a fixture is hand-written but view.rs's Counted and orphan-impl.rs's)"
    );
}
