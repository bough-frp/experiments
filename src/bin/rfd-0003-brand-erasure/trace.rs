//! The `trace` mode: three follow-ups on the derive of the `derive` and
//! `borrow` modes.
//!
//! 1. Can `Trace` be derived beside `Rebrand`, from the same attributes, and
//!    can the derive refuse a `#[rebrand(skip)]` field that holds a token? A
//!    skipped field is cloned across brands and not traced, so a token in it
//!    is invisible to the collector and goes stale.
//! 2. Can a generic type instantiated brand-free (`Tagged<u32>`) be read
//!    without a copy, by a `'static` downcast, an associated const or a
//!    marker bound, on stable and without `unsafe`?
//! 3. Can the stash route (a hand-written `view` or `Borrow` copying the
//!    `'static` token it is lent into a thread-local) be closed by a trait
//!    only the derive implements, safe or `unsafe`, or by lending the stored
//!    value at a brand local to the loan? And does an `unsafe` one survive
//!    `#![forbid(unsafe_code)]` in the user's crate?
//!
//! The derive is built by Cargo with a different feature set per library
//! (see its `Cargo.toml`), and fixtures under
//! `fixtures/rfd-0003-brand-erasure/trace/` are compiled against these, each
//! a crate of its own:
//!
//! - `bough`: `trace/bough.rs`, the main run's `api.rs` with question 2's
//!   reads appended (the binary checks it is unchanged above them), and the
//!   derive with `trace`.
//! - `plain`: `derive/bough.rs` and the derive with no feature: the
//!   committed derive, for the one row that shows what it let through.
//! - `seal`, `unsafe`, `bare`: `trace/seal.rs`, a model Bough whose
//!   `Rebrand` has a seal as a supertrait, safe or `unsafe` (`--cfg
//!   unsafe_seal`), and the derive implementing it; `bare` is the `unsafe`
//!   seal with no `#[allow(unsafe_code)]` on the derive's impl.
//! - `none`: a standalone fixture.
//!
//! Unlike the other modes, nothing here is compiled with `--cap-lints
//! allow`: that caps `forbid` too, and question 3 is about what `forbid`
//! does. So every `#![forbid(unsafe_code)]` in this mode is enforced.

use std::path::{Path, PathBuf};

use super::derive::build_derive;
use super::{Mode, Outcome, first_diagnostic, run, rustc, version};
use Expect::{Build, Fail, Refuse, Try};
use Mode::{Check, Run};

const DIR: &str = "fixtures/rfd-0003-brand-erasure/trace";

#[derive(Clone, Copy, PartialEq)]
enum Expect {
    Build,
    Fail,
    /// A route to a hidden or stashed token, which builds today.
    Refuse,
    /// An attempt at a copy-free `view` (question 2), with no expectation.
    Try,
}

/// (question, library, fixture, expected, whether it runs). A fixture of
/// `-` is the library itself, built with `--cfg core_forbid`.
const CASES: &[(u8, &str, &str, Expect, Mode)] = &[
    // Question 1: `Trace` derived, and skipped fields that hold tokens.
    (1, "bough", "traced", Build, Run),
    (1, "plain", "skip-static", Refuse, Run),
    (1, "bough", "skip-static", Fail, Check),
    (1, "bough", "skip-nested", Fail, Check),
    (1, "bough", "skip-untyped", Fail, Check),
    (1, "bough", "skip-brand", Fail, Check),
    (1, "bough", "skip-param", Fail, Check),
    (1, "bough", "skip-plain", Fail, Check),
    (1, "bough", "trace-anchored", Fail, Check),
    (1, "bough", "skip-hidden", Refuse, Check),
    (1, "bough", "leaf-static", Refuse, Check),
    // Question 2: copy-free reads of generic brand-free types.
    (2, "bough", "view-downcast", Try, Check),
    (2, "bough", "view-const", Try, Check),
    (2, "bough", "view-marker", Try, Check),
    (2, "bough", "view-marker-impl", Try, Check),
    (2, "bough", "view-marker-own", Try, Check),
    (2, "bough", "plain-read", Build, Run),
    (2, "bough", "plain-read-token", Fail, Check),
    (2, "bough", "static-read-token", Fail, Check),
    // Question 3: the stash route against a seal, and a scoped loan.
    (3, "seal", "seal-derive", Build, Run),
    (3, "seal", "seal-hand", Fail, Check),
    (3, "seal", "seal-forged", Refuse, Check),
    (3, "seal", "seal-leaf", Fail, Check),
    (3, "unsafe", "-", Build, Check),
    (3, "unsafe", "seal-derive", Build, Check),
    (3, "unsafe", "unsafe-deny", Build, Check),
    (3, "bare", "seal-derive", Build, Run),
    (3, "bare", "unsafe-hand-forbid", Fail, Check),
    (3, "bare", "unsafe-hand", Refuse, Check),
    (3, "bare", "unsafe-macro", Refuse, Check),
    (3, "none", "loan-lent", Fail, Check),
    (3, "none", "loan-covariant", Build, Check),
];

/// A Bough a fixture is compiled against.
struct Library {
    name: &'static str,
    rlib: PathBuf,
    /// The directory its proc macro was built in, for `-L`.
    deps: PathBuf,
    /// Its source and cfgs, to build it again with `core_forbid`.
    source: String,
    cfgs: &'static [&'static str],
    derive: PathBuf,
}

/// Build one library with the pinned `rustc`, into `out/<dir>/`.
fn library(
    root: &Path,
    out: &Path,
    dir: &str,
    (source, name, cfgs): (&str, &str, &[&str]),
    derive: Option<&Path>,
) -> (PathBuf, Outcome) {
    let dir = out.join(dir);
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
    for cfg in cfgs {
        command.args(["--cfg", cfg]);
    }
    if let Some(derive) = derive {
        command
            .arg("--extern")
            .arg(format!("rfd_0003_rebrand_derive={}", derive.display()));
    }
    let output = command
        .args(["--color", "never", "-o"])
        .arg(&rlib)
        .arg(Path::new(source))
        .output()
        .expect("rustc runs");
    let outcome = Outcome {
        built: output.status.success(),
        first: first_diagnostic(&String::from_utf8_lossy(&output.stderr)),
        ran: None,
    };
    (rlib, outcome)
}

fn fixture(
    root: &Path,
    out: &Path,
    lib: Option<&Library>,
    helper: &Path,
    fixture: &str,
    mode: Mode,
) -> Outcome {
    let binary = out.join(format!("{}-{fixture}", lib.map_or("none", |l| l.name)));
    let mut command = rustc(root, None);
    command
        .args(["--edition", "2024", "--crate-type", "bin", "--crate-name"])
        .arg(fixture.replace('-', "_"));
    if let Some(lib) = lib {
        command
            .arg("--extern")
            .arg(format!("bough={}", lib.rlib.display()))
            .arg("-L")
            .arg(format!("dependency={}", lib.deps.display()));
    }
    if fixture == "unsafe-macro" {
        command
            .arg("--extern")
            .arg(format!("helper={}", helper.display()));
    }
    match mode {
        Check => command
            .args(["--emit=metadata", "-o"])
            .arg(binary.with_extension("rmeta")),
        Run => command.arg("-o").arg(&binary),
    };
    let output = command
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

/// `trace/api.rs` must be `../api.rs` with only question 2's reads after it.
fn check_api(root: &Path) -> (usize, usize) {
    let read = |path: &str| {
        std::fs::read_to_string(root.join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
    };
    let original = read("fixtures/rfd-0003-brand-erasure/api.rs");
    let ours = read(&format!("{DIR}/api.rs"));
    let added = ours
        .strip_prefix(original.as_str())
        .expect("trace/api.rs starts with ../api.rs, byte for byte");
    (original.lines().count(), added.lines().count())
}

pub fn main(root: &Path) {
    println!("stable:  {}  (rust-toolchain.toml)", version(root, None));
    let (kept, added) = check_api(root);
    let out = root.join("target/rfd-0003-brand-erasure/trace");
    std::fs::create_dir_all(&out).expect("the output directory");

    // (library, derive features, source, cfgs).
    let specs: [(&str, &[&str], &str, &[&str]); 5] = [
        ("bough", &["trace"], "trace/bough.rs", &[]),
        ("plain", &[], "derive/bough.rs", &[]),
        ("seal", &["seal"], "trace/seal.rs", &[]),
        (
            "unsafe",
            &["unsafe-seal"],
            "trace/seal.rs",
            &["unsafe_seal"],
        ),
        ("bare", &["bare-unsafe"], "trace/seal.rs", &["unsafe_seal"]),
    ];
    let mut libs = Vec::new();
    for (name, features, source, cfgs) in specs {
        let derive = build_derive(root, &format!("trace-build-{name}"), features);
        let source = format!("fixtures/rfd-0003-brand-erasure/{source}");
        let (rlib, outcome) = library(root, &out, name, (&source, "bough", cfgs), Some(&derive));
        assert!(outcome.built, "{source} builds: {:?}", outcome.first);
        libs.push(Library {
            name,
            rlib,
            deps: derive.parent().expect("the deps directory").to_path_buf(),
            source,
            cfgs,
            derive,
        });
    }
    let (helper, outcome) = library(
        root,
        &out,
        "helper",
        (&format!("{DIR}/helper.rs"), "helper", &[]),
        None,
    );
    assert!(outcome.built, "helper.rs builds: {:?}", outcome.first);
    println!("derive:  rfd-0003-rebrand-derive, built by cargo, once per library:");
    println!("         bough  --features trace         trace/bough.rs");
    println!("         plain  (no features)            derive/bough.rs");
    println!("         seal   --features seal          trace/seal.rs");
    println!("         unsafe --features unsafe-seal   trace/seal.rs --cfg unsafe_seal");
    println!("         bare   --features bare-unsafe   trace/seal.rs --cfg unsafe_seal");
    println!(
        "api:     trace/api.rs = ../api.rs ({kept} lines, unchanged) + {added} lines of question 2 reads"
    );
    println!("lints:   not capped: every #![forbid(unsafe_code)] is enforced");
    println!();
    println!(
        "{:<2} {:<7} {:<19} {:<8} {:<7} first error",
        "q", "bough", "fixture", "expected", "result"
    );

    // Per question: (expected, succeeded) for build, fail, refuse; built of try.
    let mut tallies = [[(0usize, 0usize); 4]; 3];
    let mut outcomes = Vec::new();
    for &(q, lib, name, expect, mode) in CASES {
        let library_row = name == "-";
        let found = libs.iter().find(|l| l.name == lib);
        let (label, outcome) = if library_row {
            let lib = found.expect("a library");
            let cfgs: Vec<&str> = lib.cfgs.iter().copied().chain(["core_forbid"]).collect();
            let (_, outcome) = library(
                root,
                &out,
                "core-forbid",
                (&lib.source, "bough", &cfgs),
                Some(&lib.derive),
            );
            ("--cfg core_forbid", outcome)
        } else {
            (name, fixture(root, &out, found, &helper, name, mode))
        };
        let (i, ok) = match expect {
            Build => (0, outcome.built),
            Fail => (1, !outcome.built),
            Refuse => (2, !outcome.built),
            Try => (3, outcome.built),
        };
        let tally = &mut tallies[usize::from(q - 1)][i];
        tally.0 += 1;
        tally.1 += usize::from(ok);
        println!(
            "{:<2} {:<7} {:<19} {:<8} {:<7} {}",
            q,
            lib,
            label,
            match expect {
                Build => "build",
                Fail => "fail",
                Refuse => "fail?",
                Try => "try",
            },
            if outcome.built { "built" } else { "failed" },
            outcome.headline(),
        );
        outcomes.push((lib, label, outcome));
    }
    println!();
    println!("fail?: a token hidden from the collector (question 1) or stashed by a");
    println!("hand-written impl (question 3); it builds, and a failure is what's asked for.");
    println!("try:   an attempt at a copy-free `view` for a generic type (question 2).");
    println!();

    for (lib, name, outcome) in &outcomes {
        if let Some(lines) = &outcome.ran {
            println!("--- {lib}: {name}.rs, run");
            for line in lines {
                println!("{line}");
            }
            println!();
        }
    }
    // Every failure's whole first diagnostic: the text a user would see.
    for (lib, name, outcome) in &outcomes {
        if !outcome.built {
            println!("--- {lib}: {name}, first diagnostic");
            for line in &outcome.first {
                println!("{line}");
            }
            println!();
        }
    }
    for (q, [build, fail, refuse, tried]) in tallies.iter().enumerate() {
        let mut line = format!("question {}:", q + 1);
        let mut parts = Vec::new();
        if build.0 > 0 {
            parts.push(format!(
                "{} of {} must-build programs built",
                build.1, build.0
            ));
        }
        if fail.0 > 0 {
            parts.push(format!(
                "{} of {} must-fail programs failed",
                fail.1, fail.0
            ));
        }
        if refuse.0 > 0 {
            parts.push(format!("{} of {} routes refused", refuse.1, refuse.0));
        }
        if tried.0 > 0 {
            parts.push(format!(
                "{} of {} copy-free view attempts built",
                tried.1, tried.0
            ));
        }
        line += &format!(" {}", parts.join(", "));
        println!("{line}");
    }
    println!(
        "(every fixture is #![forbid(unsafe_code)] but unsafe-deny.rs and unsafe-hand.rs, and every library but helper.rs, and seal.rs under unsafe_seal, which denies it)"
    );
}
