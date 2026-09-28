//! The `borrow` mode: can the borrowed views the two cost probes priced be
//! derived, and can RFD 4's `sample`, which returns `&A` from a `OnceCell`
//! memo, return `T::Ref<'_>` built from the stored `T::Of<'static>` instead,
//! with no `unsafe`? And what does each view give up against `&A` and
//! `&mut S`?
//!
//! `rfd-0003-rebrand-cost` and `rfd-0003-rebrand-write-cost` found a read
//! view (`Borrow`, `T::Ref<'_>`) and a write view (`BorrowMut`, `T::Mut<'_>`)
//! cost what gc-arena's `unsafe` cast costs, where a safe restore copies on
//! every access. Both wrote the views by hand, for fixed shapes. Here:
//!
//! - The derive, built with its `views` feature, writes both for any struct
//!   or enum, generic or not: `FooRef<'a, 'g>` and `FooMut<'a, 'g>`, each
//!   field's view its type's own.
//! - `borrow/api.rs` is the main run's `api.rs`, byte for byte, with the
//!   views appended: the traits, their impls for tokens and containers,
//!   `ListRef` and `ListMut` (with `retain` and `sort_by` whose closures see
//!   views), `Place` for a whole value, `sample_ref`, a read-through
//!   `map_cell_lazy` whose memo is a `OnceCell`, and `update`, which hands a
//!   closure the committed value in place.
//! - One change to the cost probes' trait: `borrow` takes a `Loan<'a>`, a
//!   proof scoped to the call, where they took a `Witness<'static>`. A
//!   `'static` witness in a user's impl could be kept, and with it any token
//!   restored at any brand.
//!
//! The fixtures under `fixtures/rfd-0003-brand-erasure/borrow/` are compiled
//! as a user's crate against that Bough, as the `derive` mode's are. Rows
//! marked `fail?` are routes the brand should stop and that build: a stored
//! token copied out of the stored value an impl is lent.

use std::path::{Path, PathBuf};

use super::derive::build_derive;
use super::{Mode, Outcome, first_diagnostic, run, rustc, version};
use Expect::{Build, Fail, Refuse};
use Mode::{Check, Run};

const DIR: &str = "fixtures/rfd-0003-brand-erasure/borrow";

#[derive(Clone, Copy, PartialEq)]
enum Expect {
    Build,
    Fail,
    /// A route the brand should stop, which builds today.
    Refuse,
}

/// (view, fixture, expected, whether it runs).
const CASES: &[(&str, &str, Expect, Mode)] = &[
    // Question 1: the read view, and `sample` returning it.
    ("read", "sample-struct", Build, Run),
    ("read", "sample-vec", Build, Run),
    ("read", "sample-tagged", Build, Run),
    ("read", "sample-lazy", Build, Run),
    ("read", "plain-own", Build, Run),
    ("read", "ref-across-send", Fail, Check),
    ("read", "ref-out-of-mutate", Fail, Check),
    ("read", "ref-into-mutate", Fail, Check),
    ("read", "ref-token-nested", Fail, Check),
    ("read", "ref-into-closure", Fail, Check),
    ("read", "ref-static", Fail, Check),
    ("read", "stash-loan", Fail, Check),
    ("read", "missing-view", Fail, Check),
    ("read", "stash-in-borrow", Refuse, Run),
    ("read", "stash-in-view", Refuse, Run),
    // What the read view gives up against `&A`.
    ("read", "lost-index", Fail, Check),
    ("read", "lost-slice", Fail, Check),
    ("read", "lost-debug", Fail, Check),
    ("read", "lost-variance", Fail, Check),
    // Question 2: the write view.
    ("write", "mut-struct", Build, Run),
    ("write", "mut-generic", Build, Run),
    ("write", "mut-get-static", Fail, Check),
    ("write", "mut-retain-static", Fail, Check),
    ("write", "mut-sort-static", Fail, Check),
    ("write", "mut-place-static", Fail, Check),
    ("write", "mut-items", Fail, Check),
    ("write", "mut-escape", Fail, Check),
    // What the write view gives up against `&mut S`.
    ("write", "lost-variant", Fail, Check),
    ("write", "lost-dedup", Fail, Check),
];

struct Libraries {
    out: PathBuf,
    /// The directory the proc macro was built in, for `-L`.
    deps: PathBuf,
    bough: PathBuf,
}

/// Build Bough with the pinned `rustc`, the derive built with `views`.
fn library(root: &Path, out: &Path, derive: &Path) -> (PathBuf, Outcome) {
    let rlib = out.join("libbough.rlib");
    let output = rustc(root, None)
        .args(["--edition", "2024", "--crate-type", "rlib", "--crate-name"])
        .arg("bough")
        .arg("--extern")
        .arg(format!("rfd_0003_rebrand_derive={}", derive.display()))
        .args(["--cap-lints", "allow", "--color", "never", "-o"])
        .arg(&rlib)
        .arg(Path::new(DIR).join("bough.rs"))
        .output()
        .expect("rustc runs");
    let outcome = Outcome {
        built: output.status.success(),
        first: first_diagnostic(&String::from_utf8_lossy(&output.stderr)),
        ran: None,
    };
    (rlib, outcome)
}

fn fixture(root: &Path, libs: &Libraries, fixture: &str, mode: Mode) -> Outcome {
    let binary = libs.out.join(fixture);
    let mut command = rustc(root, None);
    command
        .args(["--edition", "2024", "--crate-type", "bin", "--crate-name"])
        .arg(fixture.replace('-', "_"))
        .arg("--extern")
        .arg(format!("bough={}", libs.bough.display()))
        .arg("-L")
        .arg(format!("dependency={}", libs.deps.display()));
    match mode {
        Check => command
            .args(["--emit=metadata", "-o"])
            .arg(binary.with_extension("rmeta")),
        Run => command.arg("-o").arg(&binary),
    };
    let output = command
        .args(["--cap-lints", "allow", "--color", "never"])
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

/// `borrow/api.rs` must be `../api.rs` with only the views after it.
fn check_api(root: &Path) -> (usize, usize) {
    let read = |path: &str| {
        std::fs::read_to_string(root.join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
    };
    let original = read("fixtures/rfd-0003-brand-erasure/api.rs");
    let ours = read(&format!("{DIR}/api.rs"));
    let views = ours
        .strip_prefix(original.as_str())
        .expect("borrow/api.rs starts with ../api.rs, byte for byte");
    (original.lines().count(), views.lines().count())
}

pub fn main(root: &Path) {
    println!("stable:  {}  (rust-toolchain.toml)", version(root, None));
    let (kept, added) = check_api(root);
    let derive = build_derive(root, "borrow-build", &["views"]);
    let out = root.join("target/rfd-0003-brand-erasure/borrow");
    std::fs::create_dir_all(&out).expect("the output directory");
    let (bough, outcome) = library(root, &out, &derive);
    assert!(outcome.built, "borrow/bough.rs builds: {:?}", outcome.first);
    let libs = Libraries {
        deps: derive.parent().expect("the deps directory").to_path_buf(),
        bough,
        out,
    };
    println!("derive:  rfd-0003-rebrand-derive --features views, built by cargo");
    println!(
        "api:     borrow/api.rs = ../api.rs ({kept} lines, unchanged) + {added} lines of views"
    );
    println!();
    println!(
        "{:<6} {:<18} {:<8} {:<7} first error",
        "view", "fixture", "expected", "result"
    );

    let mut tally = [(0, 0); 3];
    let mut outcomes = Vec::new();
    for &(view, name, expect, mode) in CASES {
        let outcome = fixture(root, &libs, name, mode);
        let (i, ok) = match expect {
            Build => (0, outcome.built),
            Fail => (1, !outcome.built),
            Refuse => (2, !outcome.built),
        };
        tally[i].0 += 1;
        tally[i].1 += usize::from(ok);
        println!(
            "{:<6} {:<18} {:<8} {:<7} {}",
            view,
            name,
            match expect {
                Build => "build",
                Fail => "fail",
                Refuse => "fail?",
            },
            if outcome.built { "built" } else { "failed" },
            outcome.headline(),
        );
        outcomes.push((view, name, outcome));
    }
    println!();
    println!("fail?: a token copied out of the stored value a hand-written impl is lent;");
    println!("it builds, and the run shows the token outliving its root.");
    println!();

    for (view, name, outcome) in &outcomes {
        if let Some(lines) = &outcome.ran {
            println!("--- {view}: {name}.rs, run");
            for line in lines {
                println!("{line}");
            }
            println!();
        }
    }
    // Every failure's whole first diagnostic: the text a user would see.
    for (view, name, outcome) in &outcomes {
        if !outcome.built {
            println!("--- {view}: {name}.rs, first diagnostic");
            for line in &outcome.first {
                println!("{line}");
            }
            println!();
        }
    }
    let [(builds, built), (fails, failed), (refuses, refused)] = tally;
    println!(
        "borrow: {built} of {builds} must-build programs built, {failed} of {fails} must-fail programs failed, {refused} of {refuses} stash routes refused"
    );
    println!(
        "(every fixture and library is #![forbid(unsafe_code)]; no view in a fixture is hand-written but sample-struct.rs's Counted and the stash fixtures')"
    );
}
