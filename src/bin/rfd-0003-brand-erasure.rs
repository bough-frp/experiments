//! Can gc-arena's brand be erased and restored without `unsafe`, does RFD 6's
//! `RemoteIo` survive it, and can the leak route through a captured
//! `Anchored` be made a compile error?
//!
//! `rfd-0003-branded-captures` showed that a brand on every token makes F62's
//! forgotten capture a compile error, on a skeleton whose `open` was
//! `todo!()`. This probe fills it in. `fixtures/rfd-0003-brand-erasure/api.rs`
//! is a real toy runtime under `#![forbid(unsafe_code)]`: tokens are indices
//! into an arena, `Rebrand` rebuilds a value at another brand field by field,
//! with a witness only the runtime can mint, and stored closures run at the
//! brand they were built with, behind wrappers that capture nothing branded.
//! Four designs share the fixtures:
//!
//! - `erasure`: that runtime, with `anchor` and `open` on the `Build` graph
//!   code gets, as in the earlier probe.
//! - `split`: the same, `--cfg bough_split`, with `anchor`, `open` and the
//!   other I/O methods only on the `Mutate` a `mutate` gets.
//! - `auto`: the same, `--cfg bough_auto` on the dated nightly, with every
//!   graph closure bound by an auto trait that `Anchored` opts out of.
//! - `scoped`: types only, `scoped/api.rs`, where `Anchored` carries a
//!   second, per-runtime brand, so it isn't `'static`.
//!
//! The binary compiles each (design, fixture) pair with the pinned `rustc`
//! (the nightly for `auto`), prints whether it built and its first error,
//! runs the fixtures marked to run, and prints their output.
//!
//! `-- derive` runs a follow-up instead, in `rfd-0003-brand-erasure/derive.rs`:
//! the same `api.rs` with its `Rebrand` impls written by a real proc macro.
//! `-- borrow` runs another, in `rfd-0003-brand-erasure/borrow.rs`: that
//! derive also writing borrowed read and write views, and `sample` returning
//! one.

#[path = "rfd-0003-brand-erasure/borrow.rs"]
mod borrow;
#[path = "rfd-0003-brand-erasure/derive.rs"]
mod derive;

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The nightly the auto-trait design needs, pinned by date, as in the
/// earlier probe.
const NIGHTLY: &str = "nightly-2026-09-22";

const DIR: &str = "fixtures/rfd-0003-brand-erasure";

struct Design {
    name: &'static str,
    /// Under `DIR`; the scoped design has its own `api.rs`.
    subdir: &'static str,
    cfg: Option<&'static str>,
    toolchain: Option<&'static str>,
}

const DESIGNS: [Design; 4] = [
    Design {
        name: "erasure",
        subdir: "",
        cfg: None,
        toolchain: None,
    },
    Design {
        name: "split",
        subdir: "",
        cfg: Some("bough_split"),
        toolchain: None,
    },
    Design {
        name: "auto",
        subdir: "",
        cfg: Some("bough_auto"),
        toolchain: Some(NIGHTLY),
    },
    Design {
        name: "scoped",
        subdir: "scoped",
        cfg: None,
        toolchain: None,
    },
];

#[derive(Clone, Copy, PartialEq)]
enum Expect {
    Build,
    Fail,
    /// A leak route: builds today, and question 3 asks whether it can fail.
    Refuse,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Check,
    Run,
}

use Expect::{Build, Fail, Refuse};
use Mode::{Check, Run};

/// (design, fixture, expected, whether it runs).
const CASES: &[(&str, &str, Expect, Mode)] = &[
    // Question 1: the safe runtime, the programs it must still take, and
    // the misuses it must refuse.
    ("erasure", "derive-shapes", Build, Run),
    ("erasure", "wrong-rebrand", Build, Run),
    ("erasure", "screens", Build, Check),
    ("erasure", "screens-io", Build, Check),
    ("erasure", "switch-choice", Build, Check),
    ("erasure", "generic-helper", Build, Check),
    ("erasure", "forgot-capture", Fail, Check),
    ("erasure", "static-token", Fail, Check),
    ("erasure", "mutate-return", Fail, Check),
    ("erasure", "forge-witness", Fail, Check),
    ("erasure", "stash-in-impl", Fail, Check),
    ("erasure", "identity-of", Fail, Check),
    ("erasure", "witness-capture", Fail, Check),
    // Question 2: RFD 6's handles.
    ("erasure", "remote-chat", Build, Run),
    ("erasure", "remote-listen", Build, Run),
    ("erasure", "remote-listen-leak", Fail, Check),
    ("erasure", "remote-raw-token", Fail, Check),
    // Question 3: the leak route, in every design.
    ("erasure", "leak-open", Refuse, Run),
    ("erasure", "leak-hold", Refuse, Check),
    ("erasure", "leak-shared", Refuse, Check),
    ("split", "derive-shapes", Build, Check),
    ("split", "screens", Build, Check),
    ("split", "screens-io", Build, Check),
    ("split", "switch-choice", Build, Check),
    ("split", "remote-chat", Build, Run),
    ("split", "remote-listen", Build, Check),
    ("split", "leak-open", Refuse, Check),
    ("split", "leak-hold", Refuse, Check),
    ("split", "leak-shared", Refuse, Check),
    ("auto", "derive-shapes", Build, Check),
    ("auto", "screens", Build, Check),
    ("auto", "screens-io", Build, Check),
    ("auto", "switch-choice", Build, Check),
    ("auto", "generic-helper", Build, Check),
    ("auto", "remote-chat", Build, Check),
    ("auto", "remote-listen", Build, Check),
    ("auto", "forgot-capture", Fail, Check),
    ("auto", "leak-open", Refuse, Check),
    ("auto", "leak-hold", Refuse, Check),
    ("auto", "leak-shared", Refuse, Check),
    ("scoped", "anchor", Build, Check),
    ("scoped", "remote-spawn", Build, Check),
    ("scoped", "remote-scoped", Build, Check),
    ("scoped", "leak-open", Refuse, Check),
    ("scoped", "leak-shared", Refuse, Check),
];

/// The (design, fixture) pairs whose whole first diagnostic is printed.
const SHOWN: &[(&str, &str)] = &[
    ("erasure", "stash-in-impl"),
    ("erasure", "identity-of"),
    ("split", "leak-open"),
    ("auto", "leak-open"),
    ("scoped", "leak-open"),
    ("scoped", "remote-spawn"),
];

struct Outcome {
    built: bool,
    /// The whole of the first diagnostic, as a user would see it.
    first: Vec<String>,
    /// What the fixture printed, for one that runs.
    ran: Option<Vec<String>>,
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

/// Run a built fixture, killing it if it hangs: a queue that never drains
/// would otherwise hold the probe forever.
fn run(binary: &Path) -> Vec<String> {
    let mut child = Command::new(binary)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the fixture starts");
    let start = Instant::now();
    loop {
        if child.try_wait().expect("the fixture's status").is_some() {
            break;
        }
        if start.elapsed() > Duration::from_secs(60) {
            child.kill().expect("the fixture is killed");
            return vec!["(killed after 60 s)".to_string()];
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = child.wait_with_output().expect("the fixture's output");
    let mut lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    if !output.status.success() {
        lines.push(format!("(exited with {})", output.status));
    }
    lines
}

/// The whole of the first error in `rustc`'s output.
fn first_diagnostic(stderr: &str) -> Vec<String> {
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
    first
}

fn compile(root: &Path, design: &Design, fixture: &str, mode: Mode) -> Outcome {
    let source = Path::new(DIR)
        .join(design.subdir)
        .join(format!("{fixture}.rs"));
    let out = root.join("target/rfd-0003-brand-erasure");
    std::fs::create_dir_all(&out).expect("the output directory");
    let binary = out.join(format!("{}-{fixture}", design.name));
    let mut command = rustc(root, design.toolchain);
    command.args(["--edition", "2024", "--crate-type", "bin"]);
    if let Some(cfg) = design.cfg {
        command.args(["--cfg", cfg]);
    }
    match mode {
        // Type and borrow checking, no codegen.
        Check => command
            .args(["--emit=metadata", "-o"])
            .arg(binary.with_extension("rmeta")),
        Run => command.arg("-o").arg(&binary),
    };
    // Warnings are noise here; errors are what the probe reads.
    let output = command
        .args(["--cap-lints", "allow", "--color", "never"])
        .arg(&source)
        .output()
        .expect("rustc runs");
    let first = first_diagnostic(&String::from_utf8_lossy(&output.stderr));
    let built = output.status.success();
    let ran = (built && mode == Run).then(|| run(&binary));
    Outcome { built, first, ran }
}

#[derive(Default)]
struct Tally {
    builds: usize,
    built: usize,
    fails: usize,
    failed: usize,
    refuses: usize,
    refused: usize,
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    // `-- derive` runs the follow-up on a real derive instead.
    if std::env::args().nth(1).as_deref() == Some("derive") {
        derive::main(root);
        return;
    }
    // `-- borrow` runs the follow-up on derived borrowed views.
    if std::env::args().nth(1).as_deref() == Some("borrow") {
        borrow::main(root);
        return;
    }
    println!("stable:  {}  (rust-toolchain.toml)", version(root, None));
    println!(
        "nightly: {}  (pinned as {NIGHTLY}, auto only)",
        version(root, Some(NIGHTLY))
    );
    println!();
    println!(
        "{:<8} {:<19} {:<8} {:<7} first error",
        "design", "fixture", "expected", "result"
    );

    let mut shown = Vec::new();
    let mut runs = Vec::new();
    let mut tallies = Vec::new();
    for design in &DESIGNS {
        let mut tally = Tally::default();
        for &(name, fixture, expect, mode) in CASES {
            if name != design.name {
                continue;
            }
            let outcome = compile(root, design, fixture, mode);
            match expect {
                Build => {
                    tally.builds += 1;
                    tally.built += usize::from(outcome.built);
                }
                Fail => {
                    tally.fails += 1;
                    tally.failed += usize::from(!outcome.built);
                }
                Refuse => {
                    tally.refuses += 1;
                    tally.refused += usize::from(!outcome.built);
                }
            }
            println!(
                "{:<8} {:<19} {:<8} {:<7} {}",
                design.name,
                fixture,
                match expect {
                    Build => "build",
                    Fail => "fail",
                    Refuse => "fail?",
                },
                if outcome.built { "built" } else { "failed" },
                outcome.headline(),
            );
            if let Some(lines) = &outcome.ran {
                runs.push((design.name, fixture, lines.clone()));
            }
            if SHOWN.contains(&(design.name, fixture)) {
                shown.push((design.name, fixture, outcome));
            }
        }
        tallies.push((design.name, tally));
        println!();
    }

    println!("fail?: a leak route (question 3), a guard hidden in graph state; it isn't");
    println!("unsound, and a failure is what the question asks for.");
    println!();
    for (design, fixture, lines) in &runs {
        println!("--- {design}: {fixture}.rs, run");
        for line in lines {
            println!("{line}");
        }
        println!();
    }
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

    for (design, t) in &tallies {
        let mut line = format!(
            "{design}: {} of {} must-build programs built",
            t.built, t.builds
        );
        if t.fails > 0 {
            line += &format!(", {} of {} must-fail programs failed", t.failed, t.fails);
        }
        line += &format!(", {} of {} leak routes refused", t.refused, t.refuses);
        println!("{line}");
    }
    println!("(every fixture and both api.rs are #![forbid(unsafe_code)])");
}
