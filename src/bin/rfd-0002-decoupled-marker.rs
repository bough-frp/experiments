//! Does a one-bit decoupledness marker make F3 a compile error while F1's
//! counter still compiles, and what does it cost at compile time?
//!
//! RFD 2's old loop rule accepted F3, `c = hold 0 (merge ticks (map (+1)
//! (steps c)))`, a same-instant cycle through a hold's steps view; the
//! engine now refuses it at run time, when the loop closes, by checking the
//! dependency graph stays acyclic. Keating and Gale's decoupledness bit,
//! typed as Sculthorpe and Nilsson do, would refuse it at compile time.
//! This probe builds that bit into a small stream/cell DSL on stable Rust
//! and compiles a set of loops against it.
//!
//! The designs are API files under `fixtures/rfd-0002-decoupled-marker/api/`:
//!
//! - `baseline`: the DSL with no mark, RFD 2 as written. Every fixture
//!   builds, so a fixture that fails here is broken rather than refused.
//! - `marker`: one bit per stream and cell type, `Decoupled` or
//!   `Instantaneous`; `close` requires `Decoupled`.
//! - `rows`: Cuoq and Pouzet's presence rows as a four-slot type-level
//!   bitset, the fallback for what the bit is too coarse for.
//!
//! Each fixture is a program with a `//@ legal yes|no` line, whether
//! Bough's run-time rule accepts it, and a `//@ designs` line. The
//! baseline and marker share their programs: a fixture picks the API with
//! `cfg_attr(design = ...)`, so the two compile the same text. The binary
//! compiles each with the pinned `rustc` (`--emit=metadata`: type checking,
//! no codegen) and prints whether it built and the first error.
//!
//! `--compile-time` times the baseline against the marker instead: every
//! fixture both build, and a generated program of every depth-three chain
//! over `map`, `filter`, `snapshot` and `merge`, each closing its own loop,
//! with the rows design alongside for the generated program. Each is type
//! checked and built at opt-level 0 and 3, several times, interleaved, and
//! the binary prints the ratio of medians to the baseline with a bootstrap
//! 95% interval. It is wall-clock, so its numbers count only on an idle
//! machine.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

const NAME: &str = "rfd-0002-decoupled-marker";

#[derive(Clone, Copy, PartialEq)]
enum Design {
    Baseline,
    Marker,
    Rows,
}

impl Design {
    fn name(self) -> &'static str {
        match self {
            Design::Baseline => "baseline",
            Design::Marker => "marker",
            Design::Rows => "rows",
        }
    }

    fn parse(name: &str) -> Design {
        match name {
            "baseline" => Design::Baseline,
            "marker" => Design::Marker,
            "rows" => Design::Rows,
            other => panic!("unknown design {other}"),
        }
    }
}

struct Fixture {
    name: String,
    path: PathBuf,
    legal: bool,
    designs: Vec<Design>,
}

/// Reads every fixture and its `//@` lines, in name order.
fn fixtures(root: &Path) -> Vec<Fixture> {
    // Relative to the root, so diagnostics show the paths a user would.
    let dir = Path::new("fixtures").join(NAME);
    let mut paths = std::fs::read_dir(root.join(&dir))
        .expect("the fixture directory")
        .map(|entry| dir.join(entry.expect("a directory entry").file_name()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(root.join(&path)).expect("a fixture");
            let mut legal = None;
            let mut designs = Vec::new();
            for line in text.lines() {
                if let Some(rest) = line.strip_prefix("//@ legal ") {
                    legal = Some(rest.trim() == "yes");
                }
                if let Some(rest) = line.strip_prefix("//@ designs ") {
                    designs = rest.split_whitespace().map(Design::parse).collect();
                }
            }
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            Fixture {
                legal: legal.unwrap_or_else(|| panic!("{name} has no `//@ legal` line")),
                name,
                path,
                designs,
            }
        })
        .collect()
}

#[derive(Clone, Copy)]
enum Emit {
    Check,
    Debug,
    Release,
}

impl Emit {
    fn name(self) -> &'static str {
        match self {
            Emit::Check => "check (--emit=metadata)",
            Emit::Debug => "build (--emit=link, opt-level 0)",
            Emit::Release => "build (--emit=link, opt-level 3)",
        }
    }
}

struct Outcome {
    built: bool,
    /// The whole of the first diagnostic, as a user would see it.
    first: Vec<String>,
}

impl Outcome {
    /// `E0277 this loop's definition ...`, from the first diagnostic's
    /// first line.
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

fn out_dir(root: &Path) -> PathBuf {
    let out = root.join("target").join(NAME);
    std::fs::create_dir_all(&out).expect("the output directory");
    out
}

/// Compiles one source with the pinned `rustc`: from the repository root,
/// `rustc` resolves through rust-toolchain.toml.
fn compile(root: &Path, source: &Path, design: Design, emit: Emit) -> Outcome {
    let stem = source.file_stem().unwrap().to_string_lossy();
    let mut command = Command::new("rustc");
    command
        .current_dir(root)
        .args(["--edition", "2024", "--crate-type", "lib"])
        .arg("--cfg")
        .arg(format!("design=\"{}\"", design.name()))
        // Warnings are noise here; errors are what the probe reads.
        .args(["--cap-lints", "allow", "--color", "never"]);
    match emit {
        Emit::Check => command.arg("--emit=metadata"),
        Emit::Debug => command.args(["--emit=link", "-C", "opt-level=0"]),
        Emit::Release => command.args(["--emit=link", "-C", "opt-level=3"]),
    };
    let output = command
        .arg("--crate-name")
        .arg(format!("{}_{}", design.name(), stem.replace('-', "_")))
        .arg("--out-dir")
        .arg(out_dir(root))
        .arg(source)
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

fn version(root: &Path) -> String {
    let output = Command::new("rustc")
        .current_dir(root)
        .arg("-V")
        .output()
        .expect("rustc runs");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

// ----- fixtures mode -----

/// The fixtures whose full first diagnostic is printed: F3 as the user
/// meets it, a stream loop, the two helper signatures, and the rows'
/// declaration refusal.
const SHOWN: [(&str, Design); 5] = [
    ("f3-steps-cycle", Design::Marker),
    ("map-only", Design::Marker),
    ("helper-opaque", Design::Marker),
    ("helper-token", Design::Marker),
    ("rows-two-loops-undeclared", Design::Rows),
];

/// What one checked design got wrong against the run-time rule.
#[derive(Default)]
struct Tally {
    legal: usize,
    illegal: usize,
    false_refusals: Vec<String>,
    false_acceptances: Vec<String>,
}

fn run_fixtures(root: &Path) {
    println!("rustc: {}  (rust-toolchain.toml)", version(root));
    println!("check: rustc --emit=metadata per design; the baseline must build every fixture,");
    println!("       marker and rows must build exactly the legal loops");
    println!();
    println!(
        "{:<27} {:<6} {:<9} {:<9} {:<9} first error (marker, else rows)",
        "fixture", "legal", "baseline", "marker", "rows"
    );

    let mut shown = Vec::new();
    let mut marker = Tally::default();
    let mut rows = Tally::default();
    let mut baseline_broken = Vec::new();
    let (mut f1_built, mut f3_refused) = (false, false);
    for fixture in fixtures(root) {
        let mut cells = Vec::new();
        let mut headline = String::new();
        for design in [Design::Baseline, Design::Marker, Design::Rows] {
            if !fixture.designs.contains(&design) {
                cells.push("-".to_string());
                continue;
            }
            let outcome = compile(root, &fixture.path, design, Emit::Check);
            let expected = design == Design::Baseline || fixture.legal;
            // A star marks a result that disagrees with the run-time rule.
            let star = if outcome.built == expected { "" } else { " *" };
            let word = if outcome.built { "built" } else { "refused" };
            cells.push(format!("{word}{star}"));
            let tally = match design {
                Design::Baseline => {
                    if !outcome.built {
                        baseline_broken.push(fixture.name.clone());
                    }
                    None
                }
                Design::Marker => Some(&mut marker),
                Design::Rows => Some(&mut rows),
            };
            if let Some(tally) = tally {
                if fixture.legal {
                    tally.legal += 1;
                    if !outcome.built {
                        tally.false_refusals.push(fixture.name.clone());
                    }
                } else {
                    tally.illegal += 1;
                    if outcome.built {
                        tally.false_acceptances.push(fixture.name.clone());
                    }
                }
                if headline.is_empty() {
                    headline = outcome.headline();
                }
            }
            if design == Design::Marker {
                match fixture.name.as_str() {
                    "f1-counter" => f1_built = outcome.built,
                    "f3-steps-cycle" => f3_refused = !outcome.built,
                    _ => {}
                }
            }
            if SHOWN.contains(&(fixture.name.as_str(), design)) {
                shown.push((fixture.name.clone(), design, outcome));
            }
        }
        println!(
            "{:<27} {:<6} {:<9} {:<9} {:<9} {headline}",
            fixture.name,
            if fixture.legal { "yes" } else { "no" },
            cells[0],
            cells[1],
            cells[2],
        );
    }
    println!();
    println!("* disagrees with the run-time rule: a legal loop refused, or an illegal one built");
    println!();

    for (fixture, design, outcome) in &shown {
        println!("--- {}: {fixture}.rs, first diagnostic", design.name());
        if outcome.built {
            println!("(built: no diagnostic)");
        }
        for line in &outcome.first {
            println!("{line}");
        }
        println!();
    }

    if !baseline_broken.is_empty() {
        println!("the baseline refused {baseline_broken:?}: those fixtures are broken");
    }
    for (design, tally) in [("marker", marker), ("rows", rows)] {
        println!(
            "{design}: built {} of {} legal loops, refused {} of {} illegal ones",
            tally.legal - tally.false_refusals.len(),
            tally.legal,
            tally.illegal - tally.false_acceptances.len(),
            tally.illegal,
        );
        println!(
            "  false refusals {:?}; false acceptances {:?}",
            tally.false_refusals, tally.false_acceptances
        );
    }
    println!(
        "verdict: the marker {} F3 and {} F1's counter",
        if f3_refused { "refused" } else { "built" },
        if f1_built { "built" } else { "refused" },
    );
}

// ----- compile-time mode -----

const OPS: [&str; 4] = ["map", "filter", "snapshot", "merge"];

/// Phase 6's defaults: ten timed runs, and 512 generated chains.
const RUNS: usize = 10;
const COPIES: usize = 8;

/// Every depth-three chain over `OPS`, `copies` times, each in its own
/// function closing its own loop. The text is the same for every design
/// but the rows' loop declaration, which reuses one slot: the loops are
/// independent.
fn generated(root: &Path, design: Design, copies: usize) -> PathBuf {
    let api = root
        .join("fixtures")
        .join(NAME)
        .join("api")
        .join(format!("{}.rs", design.name()));
    let mut text = String::new();
    writeln!(text, "#[path = {api:?}]\nmod bough;\nuse bough::*;\n").unwrap();
    let declare = match design {
        Design::Rows => "b.cell_loop::<u32, L0, Empty>()",
        Design::Baseline | Design::Marker => "b.cell_loop::<u32>()",
    };
    let mut n = 0;
    for _ in 0..copies {
        for shape in 0..OPS.len().pow(3) {
            let ops = [shape / 16, (shape / 4) % 4, shape % 4].map(|i| OPS[i]);
            writeln!(text, "// {}", ops.join(" > ")).unwrap();
            writeln!(text, "fn chain_{n}(b: &mut Build) {{").unwrap();
            writeln!(text, "    let s = b.input::<u32>();").unwrap();
            writeln!(text, "    let (c, c_loop) = {declare};").unwrap();
            for (depth, op) in ops.iter().enumerate() {
                // Distinct constants keep the closures from looking alike.
                let k = n * 3 + depth + 2;
                let line = match *op {
                    "map" => format!("let s = s.map(|x| x.wrapping_mul({k}));"),
                    "filter" => format!("let s = s.filter(|x| x % {k} != 0);"),
                    "snapshot" => format!("let s = s.snapshot(c, |x, v| x ^ v.wrapping_add({k}));"),
                    _ => format!(
                        "let o = b.input::<u32>();\n    let s = s.merge(b, o.map(|x| x + {k}));"
                    ),
                };
                writeln!(text, "    {line}").unwrap();
            }
            writeln!(text, "    let next = s.hold(b, 0);").unwrap();
            writeln!(text, "    c_loop.close(b, next);\n}}\n").unwrap();
            n += 1;
        }
    }
    writeln!(text, "pub fn program() {{\n    let mut b = Build::new();").unwrap();
    for i in 0..n {
        writeln!(text, "    chain_{i}(&mut b);").unwrap();
    }
    writeln!(text, "}}").unwrap();
    let path = out_dir(root).join(format!("generated-{}.rs", design.name()));
    std::fs::write(&path, text).expect("the generated program");
    path
}

/// A fixed-seed xorshift, for the bootstrap.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }

    fn resample(&mut self, samples: &[f64]) -> Vec<f64> {
        (0..samples.len())
            .map(|_| samples[self.below(samples.len())])
            .collect()
    }
}

fn median(samples: &[f64]) -> f64 {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

/// The ratio of medians, and a bootstrap 95% interval.
fn ratio(variant: &[f64], baseline: &[f64]) -> String {
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let mut ratios = (0..2000)
        .map(|_| median(&rng.resample(variant)) / median(&rng.resample(baseline)))
        .collect::<Vec<_>>();
    ratios.sort_by(f64::total_cmp);
    format!(
        "{:.3} [{:.3}, {:.3}]",
        median(variant) / median(baseline),
        ratios[ratios.len() * 25 / 1000],
        ratios[ratios.len() * 975 / 1000],
    )
}

/// Times each design on its source, interleaved run by run so drift hits
/// every design alike. Seconds, per design, per run.
fn time(root: &Path, sources: &[(Design, PathBuf)], emit: Emit, runs: usize) -> Vec<Vec<f64>> {
    // One untimed build each first, so no timed run is the one that warms
    // the file cache.
    for (design, source) in sources {
        let outcome = compile(root, source, *design, emit);
        assert!(outcome.built, "{source:?} builds: {:?}", outcome.first);
    }
    let mut samples = vec![Vec::new(); sources.len()];
    for _ in 0..runs {
        for (i, (design, source)) in sources.iter().enumerate() {
            let start = Instant::now();
            compile(root, source, *design, emit);
            samples[i].push(start.elapsed().as_secs_f64());
        }
    }
    samples
}

fn run_compile_time(root: &Path, runs: usize, copies: usize) {
    println!("rustc: {}  (rust-toolchain.toml)", version(root));
    println!(
        "runs: {runs} per design per mode, interleaved, after one untimed build; \
         generated program: {copies} x {} depth-three chains",
        OPS.len().pow(3)
    );
    println!("ratio: median / baseline median [bootstrap 95% interval, 2000 resamples]");
    if runs < RUNS || copies < COPIES {
        println!("reduced run (defaults are --runs {RUNS} --copies {COPIES}): indicative only");
    }
    println!();

    // The fixtures both designs build: the legal loops the marker accepts.
    let both = fixtures(root)
        .into_iter()
        .filter(|f| f.designs.contains(&Design::Baseline) && f.designs.contains(&Design::Marker))
        .filter(|f| compile(root, &f.path, Design::Marker, Emit::Check).built)
        .collect::<Vec<_>>();
    let generated = [Design::Baseline, Design::Marker, Design::Rows]
        .map(|design| (design, generated(root, design, copies)));

    for emit in [Emit::Check, Emit::Debug, Emit::Release] {
        println!("{}", emit.name());
        println!(
            "  {:<26} {:>11}  {:<24} rows / baseline",
            "program", "baseline ms", "marker / baseline"
        );
        let mut logs = Vec::new();
        for fixture in &both {
            let sources = [Design::Baseline, Design::Marker].map(|d| (d, fixture.path.clone()));
            let samples = time(root, &sources, emit, runs);
            logs.push((median(&samples[1]) / median(&samples[0])).ln());
            println!(
                "  {:<26} {:>11.1}  {:<24} -",
                fixture.name,
                median(&samples[0]) * 1000.0,
                ratio(&samples[1], &samples[0]),
            );
        }
        let geomean = (logs.iter().sum::<f64>() / logs.len() as f64).exp();
        println!(
            "  {:<26} {:>11}  {geomean:.3}",
            "fixtures, geometric mean", ""
        );

        let samples = time(root, &generated, emit, runs);
        println!(
            "  {:<26} {:>11.1}  {:<24} {}",
            "generated, depth three",
            median(&samples[0]) * 1000.0,
            ratio(&samples[1], &samples[0]),
            ratio(&samples[2], &samples[0]),
        );
        println!();
    }
    println!("absolute times are for scale only; quote the ratios, from an idle machine");
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let option = |name: &str, default: usize| {
        args.iter()
            .position(|arg| arg == name)
            .map_or(default, |i| args[i + 1].parse().expect("a count"))
    };
    if args.iter().any(|arg| arg == "--compile-time") {
        run_compile_time(root, option("--runs", RUNS), option("--copies", COPIES));
    } else {
        run_fixtures(root);
    }
}
