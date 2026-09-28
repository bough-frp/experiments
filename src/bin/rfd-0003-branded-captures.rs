//! Can a type make F62's forgotten capture a compile error while `hold`,
//! `construct` and `anchor` stay writable?
//!
//! RFD 3 says: "An undeclared capture cannot be made a compile error without
//! making tokens unusable as data, and we want the data." This probe tests
//! that sentence against four API skeletons, each a module of types and
//! signatures under `fixtures/rfd-0003-branded-captures/<design>/api.rs`:
//!
//! - `baseline`: RFD 3 as written, `'static` tokens and run-time `depends`.
//! - `brand`: gc-arena's `'gc`, an invariant generative lifetime on every
//!   token, graph code inside `Runtime::mutate`, `'static` graph closures.
//! - `era`: an invariant brand only on the tokens a `construct` mints, ST
//!   style after Jeltsch's era types; the top-level build is era `'static`.
//! - `auto-trait`: the baseline's tokens plus a nightly auto trait `Stable`
//!   that tokens opt out of, required of every graph closure.
//!
//! The branded and auto-trait designs pass captures as data through one
//! adapter, `with(env)`, which pairs each event with a traced environment.
//! Each design has the same programs, written as that design needs them:
//! some must build, some must fail. The binary compiles every one with
//! `rustc --emit=metadata` (type and borrow checking, no codegen) and prints
//! whether it built and the first error. The stable designs use the pinned
//! `rustc` from `rust-toolchain.toml`; `auto-trait` uses the dated nightly
//! below, by that exact name.

use std::path::Path;
use std::process::Command;

/// The nightly the auto-trait design needs, pinned by date.
const NIGHTLY: &str = "nightly-2026-09-22";

struct Design {
    name: &'static str,
    toolchain: Option<&'static str>,
}

const DESIGNS: [Design; 4] = [
    Design {
        name: "baseline",
        toolchain: None,
    },
    Design {
        name: "brand",
        toolchain: None,
    },
    Design {
        name: "era",
        toolchain: None,
    },
    Design {
        name: "auto-trait",
        toolchain: Some(NIGHTLY),
    },
];

#[derive(Clone, Copy, PartialEq)]
enum Expect {
    Build,
    Fail,
}

/// Every design has one file per fixture, named `<fixture>.rs`.
const FIXTURES: [(&str, Expect); 15] = [
    ("map", Expect::Build),
    ("snapshot", Expect::Build),
    ("hold-struct", Expect::Build),
    ("construct-three", Expect::Build),
    ("anchor", Expect::Build),
    ("screens", Expect::Build),
    ("switch-choice", Expect::Build),
    ("capture-foreign", Expect::Build),
    ("generic-helper", Expect::Build),
    ("forgot-capture", Expect::Fail),
    ("forgot-helper", Expect::Fail),
    ("forgot-switch", Expect::Fail),
    ("forgot-inner", Expect::Fail),
    ("map-to-token", Expect::Fail),
    ("thread-local", Expect::Fail),
];

/// The fixtures whose full diagnostic is printed for each design: F62 as
/// the spike found it, and as a helper function meets it.
const SHOWN: [&str; 2] = ["forgot-capture", "forgot-helper"];

/// The fixture that decides whether a design makes F62 a compile error.
const F62: &str = "forgot-capture";

struct Outcome {
    built: bool,
    /// The whole of the first diagnostic, as a user would see it.
    first: Vec<String>,
}

impl Outcome {
    /// `E0521 borrowed data escapes outside of closure`, from the first
    /// line of the first diagnostic.
    fn headline(&self) -> String {
        let Some(line) = self.first.first() else {
            return String::new();
        };
        let line = line.trim_start_matches("error");
        match line.strip_prefix('[') {
            Some(rest) => {
                let (code, message) = rest.split_once("]: ").unwrap_or((rest, ""));
                format!("{code} {message}")
            }
            None => line.trim_start_matches(": ").to_string(),
        }
    }
}

fn rustc(root: &Path, toolchain: Option<&str>) -> Command {
    let mut command = Command::new("rustc");
    if let Some(toolchain) = toolchain {
        command.arg(format!("+{toolchain}"));
    }
    // From the repository root, `rustc` resolves through rust-toolchain.toml.
    command.current_dir(root);
    command
}

fn version(root: &Path, toolchain: Option<&str>) -> String {
    let output = rustc(root, toolchain)
        .arg("-V")
        .output()
        .expect("rustc runs");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn compile(root: &Path, design: &Design, fixture: &str) -> Outcome {
    let source = format!(
        "fixtures/rfd-0003-branded-captures/{}/{fixture}.rs",
        design.name
    );
    let out = root.join("target/rfd-0003-branded-captures");
    std::fs::create_dir_all(&out).expect("the output directory");
    let output = rustc(root, design.toolchain)
        .args([
            "--edition",
            "2024",
            "--crate-type",
            "lib",
            "--emit=metadata",
        ])
        // Warnings are noise here; errors are what the probe reads.
        .args(["--cap-lints", "allow", "--color", "never"])
        .arg("-o")
        .arg(out.join(format!("{}-{fixture}.rmeta", design.name)))
        .arg(&source)
        .output()
        .expect("rustc runs");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut first = stderr
        .lines()
        .skip_while(|line| !line.starts_with("error"))
        .enumerate()
        .take_while(|(i, line)| *i == 0 || !line.starts_with("error"))
        .map(|(_, line)| line.to_string())
        .collect::<Vec<_>>();
    while first.last().is_some_and(|line| line.trim().is_empty()) {
        first.pop();
    }
    Outcome {
        built: output.status.success(),
        first,
    }
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    println!("stable:  {}  (rust-toolchain.toml)", version(root, None));
    println!(
        "nightly: {}  (pinned as {NIGHTLY}, auto-trait only)",
        version(root, Some(NIGHTLY))
    );
    println!();
    println!(
        "{:<11} {:<16} {:<8} {:<7} first error",
        "design", "fixture", "expected", "result"
    );

    let mut shown = Vec::new();
    let mut summary = Vec::new();
    for design in &DESIGNS {
        let (mut built_ok, mut builds, mut failed_ok, mut fails) = (0, 0, 0, 0);
        let mut caught = false;
        for (fixture, expect) in FIXTURES {
            let outcome = compile(root, design, fixture);
            match expect {
                Expect::Build => {
                    builds += 1;
                    built_ok += usize::from(outcome.built);
                }
                Expect::Fail => {
                    fails += 1;
                    failed_ok += usize::from(!outcome.built);
                }
            }
            println!(
                "{:<11} {:<16} {:<8} {:<7} {}",
                design.name,
                fixture,
                if expect == Expect::Build {
                    "build"
                } else {
                    "fail"
                },
                if outcome.built { "built" } else { "failed" },
                outcome.headline(),
            );
            if fixture == F62 {
                caught = !outcome.built;
            }
            if SHOWN.contains(&fixture) {
                shown.push((design.name, fixture, outcome));
            }
        }
        summary.push((design.name, caught, built_ok, builds, failed_ok, fails));
        println!();
    }

    println!("map-to-token: spike F94 made map_to trace its value, so a build there is");
    println!("safe; it is listed as 'fail' because F62 lists it.");
    println!();
    for (design, fixture, outcome) in &shown {
        println!("--- {design}: {fixture}.rs, first diagnostic");
        if outcome.built {
            println!("(built: no diagnostic)");
        }
        for line in &outcome.first {
            println!("{line}");
        }
        println!();
    }

    for (design, caught, built_ok, builds, failed_ok, fails) in summary {
        println!(
            "{design}: F62 a compile error: {}; {built_ok} of {builds} must-build programs \
             built, {failed_ok} of {fails} must-fail programs failed",
            if caught { "yes" } else { "no" },
        );
    }
    println!("(each design's must-build programs are written as that design needs them;");
    println!("the fixtures show what that costs)");
}
