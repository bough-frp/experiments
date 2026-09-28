//! The `entry` mode: two follow-ups on the stash route, where a hand-written
//! `Rebrand::view` or `Borrow::borrow`, lent the stored `T::Of<'static>`,
//! copies a `'static` token out of it.
//!
//! 2. If every entry that takes a value into a `mutate` or a handle requires
//!    the value at the brand it enters at, does a stashed token do nothing
//!    but hold a dead id? Does any legal program break? And can
//!    `RemoteIo::anchor`, a handle call from another thread with no brand of
//!    its own, carry the bound?
//! 3. With `Borrow` and `BorrowMut` `unsafe` to implement, and the derive
//!    writing a bare `unsafe impl` that a user's `#![forbid(unsafe_code)]`
//!    doesn't see, is `stash-in-borrow` closed?
//!
//! One file, `entry/api.rs`, is `borrow/api.rs` with each change under a
//! `--cfg` (its module doc lists them), and `entry/bough.rs` makes it a
//! crate, built once per design with the derive built to match:
//!
//! - `open`: no cfg, the committed design, as the control.
//! - `entry`: `bough_entry`. `Build::anchor` and `Build::send` require
//!   `T::Of<'g>: Same<T>`, which is `T: Rebrand<Of<'g> = T>` in a form rustc
//!   accepts (the `-eq` row is the equality, rejected); `Runtime::send` and
//!   `RemoteIo::send` require a value that is its own `Of` at every brand;
//!   `RemoteIo::anchor` requires `T::Of<'x>: Same<T>` for an `'x` it takes
//!   as a lifetime parameter.
//! - `at`: `bough_entry` and `bough_entry_at`. `RemoteIo::anchor` takes an
//!   `At<'x>` too, which a `RemoteIo` listener is handed with each event.
//! - `plain`: `bough_entry` with the derive built with no feature, for the
//!   `derive` mode's fixtures, whose types the views derive can't cover.
//! - `sealed`: `bough_seal_views`, the derive built with `bare-unsafe-views`.
//!   The `-core-forbid` row is this library keeping Bough's `forbid`.
//!
//! The stash fixtures under `fixtures/rfd-0003-brand-erasure/entry/` share
//! `stash.rs`, which stashes through `view` (`borrow/stash-in-view.rs`'s
//! route) so that it still builds against the sealed views. A fixture with
//! no directory is one of the default mode's, which include `api.rs` as a
//! module: it is copied next to `entry/api.rs` and compiled there. Every
//! fixture gets its library's cfgs too. As in the `trace` mode, nothing is
//! compiled with `--cap-lints allow`, so every `forbid` is enforced.

use std::path::{Path, PathBuf};

use super::derive::build_derive;
use super::{Mode, Outcome, first_diagnostic, run, rustc, version};
use Expect::{Build, Fail, Refuse, Try};
use Mode::{Check, Run};

const DIR: &str = "fixtures/rfd-0003-brand-erasure";

#[derive(Clone, Copy, PartialEq)]
enum Expect {
    Build,
    Fail,
    /// A stash route, which builds in the committed design.
    Refuse,
    /// A library built another way, to show what that costs.
    Try,
}

/// (question, library, fixture, expected, whether it runs). A fixture
/// starting with `-` is the library itself, built with one more cfg.
const CASES: &[(u8, &str, &str, Expect, Mode)] = &[
    // Question 2: the stash against the committed entries, as the control.
    (2, "open", "entry/stash-dead", Build, Run),
    (2, "open", "entry/stash-sample", Fail, Check),
    (2, "open", "entry/stash-anchor", Refuse, Run),
    (2, "open", "entry/stash-send", Refuse, Run),
    (2, "open", "entry/stash-runtime-send", Refuse, Run),
    (2, "open", "entry/stash-remote-send", Refuse, Run),
    (2, "open", "entry/stash-remote-anchor", Refuse, Run),
    (2, "open", "entry/stash-reinject", Refuse, Run),
    // The stash against bounded entries.
    (2, "entry", "-eq", Try, Check),
    (2, "entry", "entry/stash-dead", Build, Run),
    (2, "entry", "entry/stash-sample", Fail, Check),
    (2, "entry", "entry/stash-anchor", Refuse, Run),
    (2, "entry", "entry/stash-send", Refuse, Run),
    (2, "entry", "entry/stash-runtime-send", Refuse, Run),
    (2, "entry", "entry/stash-remote-send", Refuse, Run),
    (2, "entry", "entry/stash-remote-anchor", Refuse, Run),
    (2, "entry", "entry/stash-reinject", Refuse, Run),
    (2, "at", "entry/stash-remote-anchor", Refuse, Run),
    (2, "at", "entry/stash-reinject", Refuse, Run),
    // Legal programs: every must-build fixture of the default, `derive` and
    // `borrow` modes, against bounded entries.
    (2, "entry", "derive-shapes", Build, Check),
    // Wrong `Rebrand` impls on purpose, which the default mode builds and
    // runs to show the runtime catching them; a bound that refuses one at
    // compile time is a gain, so this row expects a failure.
    (2, "entry", "wrong-rebrand", Fail, Check),
    (2, "entry", "screens", Build, Check),
    (2, "entry", "screens-io", Build, Check),
    (2, "entry", "switch-choice", Build, Check),
    (2, "entry", "generic-helper", Build, Check),
    (2, "entry", "remote-chat", Build, Check),
    (2, "entry", "remote-listen", Build, Check),
    (2, "at", "derive-shapes", Build, Check),
    (2, "at", "wrong-rebrand", Fail, Check),
    (2, "at", "screens", Build, Check),
    (2, "at", "screens-io", Build, Check),
    (2, "at", "switch-choice", Build, Check),
    (2, "at", "generic-helper", Build, Check),
    (2, "at", "remote-chat", Build, Check),
    (2, "at", "remote-listen", Build, Check),
    (2, "at", "entry/remote-listen-at", Build, Run),
    (2, "plain", "derive/panels", Build, Check),
    (2, "plain", "derive/choice", Build, Check),
    (2, "plain", "derive/tagged", Build, Check),
    (2, "plain", "derive/view", Build, Check),
    (2, "plain", "derive/leaf", Build, Check),
    (2, "plain", "derive/bounded", Build, Check),
    (2, "entry", "borrow/sample-struct", Build, Check),
    (2, "entry", "borrow/sample-vec", Build, Check),
    (2, "entry", "borrow/sample-tagged", Build, Check),
    (2, "entry", "borrow/sample-lazy", Build, Check),
    (2, "entry", "borrow/plain-own", Build, Check),
    (2, "entry", "borrow/mut-struct", Build, Check),
    (2, "entry", "borrow/mut-generic", Build, Check),
    // Question 3: the stash routes against sealed views.
    (3, "sealed", "-core-forbid", Try, Check),
    (3, "sealed", "borrow/stash-in-borrow", Refuse, Run),
    (3, "sealed", "entry/stash-borrow-unsafe", Refuse, Run),
    (3, "sealed", "entry/stash-borrow-unforbidden", Refuse, Run),
    (3, "sealed", "borrow/stash-in-view", Refuse, Run),
    (3, "sealed", "entry/stash-reinject", Refuse, Run),
    // Legal programs: the `borrow` mode's, whose views the derive writes.
    (3, "sealed", "borrow/sample-struct", Build, Check),
    (3, "sealed", "borrow/sample-vec", Build, Check),
    (3, "sealed", "borrow/sample-tagged", Build, Check),
    (3, "sealed", "borrow/sample-lazy", Build, Check),
    (3, "sealed", "borrow/plain-own", Build, Check),
    (3, "sealed", "borrow/mut-struct", Build, Check),
    (3, "sealed", "borrow/mut-generic", Build, Check),
];

/// (expected, succeeded) for build, fail and refuse; (tried, built) for try.
type Tally = [(usize, usize); 4];

/// A Bough a fixture is compiled against.
struct Library {
    name: &'static str,
    rlib: PathBuf,
    /// The directory its proc macro was built in, for `-L`.
    deps: PathBuf,
    cfgs: &'static [&'static str],
    derive: PathBuf,
}

fn library(
    root: &Path,
    out: &Path,
    dir: &str,
    (source, name): (&str, &str),
    cfgs: &[&str],
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

fn fixture(
    root: &Path,
    out: &Path,
    lib: &Library,
    foreign: &Path,
    fixture: &str,
    mode: Mode,
) -> Outcome {
    let stem = fixture.rsplit('/').next().expect("a name");
    let binary = out.join(format!("{}-{stem}", lib.name));
    let mut command = rustc(root, None);
    command
        .args(["--edition", "2024", "--crate-type", "bin", "--crate-name"])
        .arg(stem.replace('-', "_"));
    for cfg in lib.cfgs {
        command.args(["--cfg", cfg]);
    }
    let source = if fixture.contains('/') {
        command
            .arg("--extern")
            .arg(format!("bough={}", lib.rlib.display()))
            .arg("--extern")
            .arg(format!("foreign={}", foreign.display()))
            .arg("-L")
            .arg(format!("dependency={}", lib.deps.display()));
        Path::new(DIR).join(format!("{fixture}.rs"))
    } else {
        // A default-mode fixture includes `api.rs` from its own directory:
        // give it this mode's, in a directory of its own.
        let src = out.join("src");
        std::fs::create_dir_all(&src).expect("the source directory");
        std::fs::copy(root.join(DIR).join("entry/api.rs"), src.join("api.rs"))
            .expect("api.rs is copied");
        let copy = src.join(format!("{fixture}.rs"));
        std::fs::copy(root.join(DIR).join(format!("{fixture}.rs")), &copy)
            .expect("the fixture is copied");
        copy.strip_prefix(root)
            .expect("under the root")
            .to_path_buf()
    };
    match mode {
        Check => command
            .args(["--emit=metadata", "-o"])
            .arg(binary.with_extension("rmeta")),
        Run => command.arg("-o").arg(&binary),
    };
    let output = command
        .args(["--color", "never"])
        .arg(&source)
        .output()
        .expect("rustc runs");
    let built = output.status.success();
    Outcome {
        built,
        first: first_diagnostic(&String::from_utf8_lossy(&output.stderr)),
        ran: (built && mode == Run).then(|| run(&binary)),
    }
}

/// Lines added and removed from `a` to `b`, by a longest common subsequence
/// of lines: how far `entry/api.rs` is from `borrow/api.rs`.
fn line_diff(a: &str, b: &str) -> (usize, usize) {
    let (a, b): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
    let mut row = vec![0usize; b.len() + 1];
    for x in &a {
        let mut diagonal = 0;
        for (j, y) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = if x == y {
                diagonal + 1
            } else {
                above.max(row[j])
            };
            diagonal = above;
        }
    }
    let common = row[b.len()];
    (b.len() - common, a.len() - common)
}

pub fn main(root: &Path) {
    println!("stable:  {}  (rust-toolchain.toml)", version(root, None));
    let read = |path: &str| {
        std::fs::read_to_string(root.join(DIR).join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
    };
    let (added, removed) = line_diff(&read("borrow/api.rs"), &read("entry/api.rs"));
    let out = root.join("target/rfd-0003-brand-erasure/entry");
    std::fs::create_dir_all(&out).expect("the output directory");

    // (library, derive features, cfgs).
    let specs: [(&str, &[&str], &[&str]); 5] = [
        ("open", &["views"], &[]),
        ("entry", &["views"], &["bough_entry"]),
        ("at", &["views"], &["bough_entry", "bough_entry_at"]),
        ("plain", &[], &["bough_entry"]),
        ("sealed", &["bare-unsafe-views"], &["bough_seal_views"]),
    ];
    let mut libs = Vec::new();
    for (name, features, cfgs) in specs {
        let derive = build_derive(
            root,
            &format!("entry-build-{}", features.join("-")),
            features,
        );
        let (rlib, outcome) = library(
            root,
            &out,
            name,
            ("entry/bough.rs", "bough"),
            cfgs,
            Some(&derive),
        );
        assert!(outcome.built, "{name} builds: {:?}", outcome.first);
        libs.push(Library {
            name,
            rlib,
            deps: derive.parent().expect("the deps directory").to_path_buf(),
            cfgs,
            derive,
        });
    }
    let (foreign, outcome) = library(
        root,
        &out,
        "foreign",
        ("derive/foreign.rs", "foreign"),
        &[],
        None,
    );
    assert!(outcome.built, "foreign.rs builds: {:?}", outcome.first);
    println!("derive:  rfd-0003-rebrand-derive, built by cargo, once per feature set:");
    println!("         open   --features views               entry/bough.rs");
    println!("         entry  --features views               entry/bough.rs --cfg bough_entry");
    println!(
        "         at     --features views               entry/bough.rs --cfg bough_entry --cfg bough_entry_at"
    );
    println!("         plain  (no features)                  entry/bough.rs --cfg bough_entry");
    println!(
        "         sealed --features bare-unsafe-views   entry/bough.rs --cfg bough_seal_views"
    );
    println!(
        "api:     entry/api.rs = borrow/api.rs + {added} lines - {removed} lines, all under the cfgs"
    );
    println!("lints:   not capped: every #![forbid(unsafe_code)] is enforced");
    println!();
    println!(
        "{:<2} {:<6} {:<31} {:<8} {:<7} first error",
        "q", "bough", "fixture", "expected", "result"
    );

    // Per question and library, in order of first appearance: (expected,
    // succeeded) for build, fail, refuse; built of try.
    let mut tallies: Vec<((u8, &str), Tally)> = Vec::new();
    let mut outcomes = Vec::new();
    for &(q, lib, name, expect, mode) in CASES {
        let lib = libs.iter().find(|l| l.name == lib).expect("a library");
        let (label, outcome) = match name {
            "-eq" | "-core-forbid" => {
                let extra = if name == "-eq" {
                    "bough_entry_eq"
                } else {
                    "bough_core_forbid"
                };
                let cfgs: Vec<&str> = lib.cfgs.iter().copied().chain([extra]).collect();
                let (_, outcome) = library(
                    root,
                    &out,
                    extra,
                    ("entry/bough.rs", "bough"),
                    &cfgs,
                    Some(&lib.derive),
                );
                (format!("--cfg {extra}"), outcome)
            }
            _ => (
                name.to_string(),
                fixture(root, &out, lib, &foreign, name, mode),
            ),
        };
        let (i, ok) = match expect {
            Build => (0, outcome.built),
            Fail => (1, !outcome.built),
            Refuse => (2, !outcome.built),
            Try => (3, outcome.built),
        };
        if !tallies.iter().any(|(key, _)| *key == (q, lib.name)) {
            tallies.push(((q, lib.name), [(0, 0); 4]));
        }
        let (_, counts) = tallies
            .iter_mut()
            .find(|(key, _)| *key == (q, lib.name))
            .expect("a tally");
        let tally = &mut counts[i];
        tally.0 += 1;
        tally.1 += usize::from(ok);
        println!(
            "{:<2} {:<6} {:<31} {:<8} {:<7} {}",
            q,
            lib.name,
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
        outcomes.push((lib.name, label, outcome));
    }
    println!();
    println!("fail?: a stashed token put to use, which builds in the committed design,");
    println!("or a stash that builds against the sealed views; a failure is what's asked for.");
    println!("try:   the library built another way, to show what that costs.");
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
    for ((q, lib), [build, fail, refuse, tried]) in &tallies {
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
            parts.push(format!("{} of {} stash routes refused", refuse.1, refuse.0));
        }
        if tried.0 > 0 {
            parts.push(format!(
                "{} of {} other library builds built",
                tried.1, tried.0
            ));
        }
        println!("question {q}, {lib:<6}: {}", parts.join(", "));
    }
    println!(
        "(every fixture is #![forbid(unsafe_code)] but entry/stash-borrow-unforbidden.rs, and every library but the sealed one, which denies it)"
    );
}
