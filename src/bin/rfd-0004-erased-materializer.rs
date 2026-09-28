//! Does erasing a fused chain at its materializer remove F36's per-chain
//! compile blow-up at depths two and three?
//!
//! F36: the spike's test binary builds programs from data, holding a chain
//! as a trait object per depth, so every chain shape it can make is a type,
//! and every materializer is compiled once per chain type. Nine adapter
//! kinds (a snapshot and a gate each of a `Cell` and of a `State`) over a
//! linear or a shared head gave 182 chain types at two adapters and 1,640 at
//! three, and 36 s and 367 s release builds.
//!
//! This binary generates that interpreter as a small crate per design and
//! depth, under `target/rfd-0004-erased-materializer/`, with no
//! dependencies: the engine is `src/rfd_0004_erased_materializer.rs`,
//! included verbatim, and `main.rs` enumerates every chain of up to `depth`
//! adapters over both heads, hands each to one of six materializers in turn
//! (`hold`, `accumulate`, `scan`, `node`, `share`, `listen`; the trait
//! object's vtable compiles all six for every chain type, as the spike's
//! did), runs ten transactions and prints the number of distinct chain types
//! it built and a checksum. The four designs differ only in the bound on a
//! chain, the path its adapters are called by, and the materializer module.
//!
//! It times `cargo build --release` of each crate from clean, several times,
//! interleaving the crates so drift spreads over all of them, and reports the
//! median per design against `baseline` at the same depth, the chain types,
//! the node eval functions in the binary (one per node type the compiler
//! instantiated), the binary's size, and the checksum, which must agree
//! across designs.
//!
//! `--quick`: five adapter kinds, depth two, one build each, to check that
//! it works. The default is phase 6's run: nine kinds, depths two and three,
//! three builds each. `--reps N` overrides the repetitions.
//!
//! Wall-clock: quote it only from an idle machine.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

/// The engine, included verbatim in every generated crate.
const ENGINE: &str = include_str!("../rfd_0004_erased_materializer.rs");

/// The adapter kinds, in the order a smaller run takes them: the spike's
/// nine. Each is the enum variant, the value the program makes, and the
/// arm of `adapt!` that applies it, with `P::` for the adapter's path.
const KINDS: [(&str, &str, &str); 9] = [
    (
        "Map(MapFn)",
        "Adapter::Map(Box::new(|x| x.wrapping_mul(3).wrapping_add(1)))",
        "Adapter::Map(f) => Box::new(P::map($s, f)) as $boxed",
    ),
    (
        "Filter(Predicate)",
        "Adapter::Filter(Box::new(|x: &i64| x % 3 != 0))",
        "Adapter::Filter(p) => Box::new(P::filter($s, p)) as $boxed",
    ),
    (
        "SnapshotCell(Cell<i64>, BinaryFn)",
        "Adapter::SnapshotCell(r.cell, Box::new(|x, c: &i64| x.wrapping_add(*c)))",
        "Adapter::SnapshotCell(c, f) => Box::new(P::snapshot($s, c, f)) as $boxed",
    ),
    (
        "GateCell(Cell<bool>)",
        "Adapter::GateCell(r.open_cell)",
        "Adapter::GateCell(c) => Box::new(P::gate($s, c)) as $boxed",
    ),
    (
        "FilterMap(FilterMapFn)",
        "Adapter::FilterMap(Box::new(|x| (x % 5 != 0).then_some(x / 2)))",
        "Adapter::FilterMap(f) => Box::new(P::filter_map($s, f)) as $boxed",
    ),
    (
        "MapTo(i64)",
        "Adapter::MapTo(7)",
        "Adapter::MapTo(v) => Box::new(P::map_to($s, v)) as $boxed",
    ),
    (
        "SnapshotState(State<i64>, BinaryFn)",
        "Adapter::SnapshotState(r.state, Box::new(|x, c: &i64| x ^ *c))",
        "Adapter::SnapshotState(c, f) => Box::new(P::snapshot($s, c, f)) as $boxed",
    ),
    (
        "GateState(State<bool>)",
        "Adapter::GateState(r.open_state)",
        "Adapter::GateState(c) => Box::new(P::gate($s, c)) as $boxed",
    ),
    (
        "Once",
        "Adapter::Once",
        "Adapter::Once => Box::new(P::once($s)) as $boxed",
    ),
];

const KIND_NAMES: [&str; 9] = [
    "map",
    "filter",
    "snapshot(Cell)",
    "gate(Cell)",
    "filter_map",
    "map_to",
    "snapshot(State)",
    "gate(State)",
    "once",
];

#[derive(Clone, Copy)]
struct Design {
    name: &'static str,
    /// The trait a chain is bound by, whose adapters `adapt!` calls.
    bound: &'static str,
    /// The materializers' module.
    materializers: &'static str,
    /// How a head becomes a chain.
    head: &'static str,
}

const DESIGNS: [Design; 4] = [
    Design {
        name: "baseline",
        bound: "Source",
        materializers: "engine",
        head: "Box::new(HEAD)",
    },
    Design {
        name: "boxed",
        bound: "Source",
        materializers: "engine::boxed",
        head: "Box::new(HEAD)",
    },
    Design {
        name: "fnptr",
        bound: "Source",
        materializers: "engine::fnptr",
        head: "Box::new(HEAD)",
    },
    Design {
        name: "flat",
        bound: "Flatten",
        materializers: "engine",
        head: "Box::new(Flat::new(HEAD))",
    },
];

const MAIN: &str = r#"//! Generated by `rfd-0004-erased-materializer`: design @DESIGN@, every chain
//! of up to @DEPTH@ adapters over @KINDS@ adapter kinds and two heads.

#[allow(dead_code)]
mod engine;

use engine::*;
use std::any::TypeId;
use std::collections::HashSet;
use std::rc::Rc;

use @MATERIALIZERS@ as m;

type MapFn = Box<dyn Fn(i64) -> i64>;
type Predicate = Box<dyn Fn(&i64) -> bool>;
type FilterMapFn = Box<dyn Fn(i64) -> Option<i64>>;
type BinaryFn = Box<dyn Fn(i64, &i64) -> i64>;
type ScanFn = Box<dyn Fn(i64, &i64) -> (i64, i64)>;
type ListenFn = Box<dyn FnMut(&i64)>;

const KINDS: usize = @KINDS@;
const DEPTH: u32 = @DEPTH@;

#[allow(dead_code)]
enum Adapter {
@VARIANTS@
}

/// The cells snapshots and gates read; fewer kinds read fewer.
#[allow(dead_code)]
struct Reads {
    cell: Cell<i64>,
    state: State<i64>,
    open_cell: Cell<bool>,
    open_state: State<bool>,
}

fn adapter(kind: usize, r: &Reads) -> Adapter {
    let _ = r;
    match kind {
@MAKES@
        _ => unreachable!(),
    }
}

/// Applies an adapter to a chain, boxed as the next depth's trait object.
macro_rules! adapt {
    ($s:expr, $a:expr, $boxed:ty) => {
        match $a {
@ARMS@
        }
    };
}

/// What a materializer does with a chain held as a trait object. The
/// vtable compiles every method for every chain type that is boxed.
trait Materialize {
    fn chain_type(&self) -> TypeId;
    fn hold(self: Box<Self>, b: &mut Graph, initial: i64) -> Cell<i64>;
    fn accumulate(self: Box<Self>, b: &mut Graph, initial: i64, f: BinaryFn) -> Cell<i64>;
    fn scan(self: Box<Self>, b: &mut Graph, initial: i64, f: ScanFn) -> Stream<i64>;
    fn node(self: Box<Self>, b: &mut Graph) -> Stream<i64>;
    fn share(self: Box<Self>, b: &mut Graph) -> Shared<i64>;
    fn listen(self: Box<Self>, b: &mut Graph, f: ListenFn);
}

impl<S: @BOUND@<Event = i64>> Materialize for S {
    fn chain_type(&self) -> TypeId {
        TypeId::of::<S>()
    }
    fn hold(self: Box<Self>, b: &mut Graph, initial: i64) -> Cell<i64> {
        m::hold(b, *self, initial)
    }
    fn accumulate(self: Box<Self>, b: &mut Graph, initial: i64, f: BinaryFn) -> Cell<i64> {
        m::accumulate(b, *self, initial, f)
    }
    fn scan(self: Box<Self>, b: &mut Graph, initial: i64, f: ScanFn) -> Stream<i64> {
        m::scan(b, *self, initial, f)
    }
    fn node(self: Box<Self>, b: &mut Graph) -> Stream<i64> {
        m::node(b, *self)
    }
    fn share(self: Box<Self>, b: &mut Graph) -> Shared<i64> {
        m::share(b, *self)
    }
    fn listen(self: Box<Self>, b: &mut Graph, f: ListenFn) {
        m::listen(b, *self, f)
    }
}

@CHAINS@

/// A chain at any depth.
enum AnyChain {
@ANY@
}

impl AnyChain {
    fn adapt(self, a: Adapter) -> AnyChain {
        match self {
@ANY_ADAPT@
        }
    }

    fn chain_type(&self) -> TypeId {
        match self {
@ANY_TYPE@
        }
    }

    fn materialize(self, b: &mut Graph, which: usize, cells: &mut Vec<Cell<i64>>, sink: &Rc<std::cell::Cell<i64>>) {
        match self {
@ANY_MATERIALIZE@
        }
    }
}

fn sink_fn(sink: &Rc<std::cell::Cell<i64>>) -> ListenFn {
    let sink = sink.clone();
    Box::new(move |v: &i64| sink.set(sink.get().wrapping_mul(3).wrapping_add(*v)))
}

fn materialize<C: Materialize + ?Sized>(
    c: Box<C>,
    b: &mut Graph,
    which: usize,
    cells: &mut Vec<Cell<i64>>,
    sink: &Rc<std::cell::Cell<i64>>,
) {
    match which % 6 {
        0 => cells.push(c.hold(b, 0)),
        1 => cells.push(c.accumulate(b, 0, Box::new(|e, s: &i64| s.wrapping_add(e)))),
        2 => {
            let s = c.scan(b, 0, Box::new(|e, s: &i64| (e ^ *s, s.wrapping_add(1))));
            engine::listen(b, s, sink_fn(sink));
        }
        3 => {
            let s = c.node(b);
            engine::listen(b, s, sink_fn(sink));
        }
        4 => {
            let s = c.share(b);
            engine::listen(b, s, sink_fn(sink));
        }
        _ => c.listen(b, sink_fn(sink)),
    }
}

fn main() {
    let mut b = Graph::new();
    let r = Reads {
        cell: b.constant_cell(3i64),
        state: b.constant_state(5i64),
        open_cell: b.constant_cell(true),
        open_state: b.constant_state(true),
    };
    let (shared, shared_in) = b.shared_input::<i64>();
    let mut inputs = Vec::new();
    let mut cells = Vec::new();
    let sink = Rc::new(std::cell::Cell::new(0i64));
    let mut types = HashSet::new();
    let mut which = 0;
    for linear in [true, false] {
        for len in 0..=DEPTH {
            for code in 0..KINDS.pow(len) {
                let head: Box<dyn Chain0> = if linear {
                    let (s, i) = b.input::<i64>();
                    inputs.push(i);
                    @HEAD_LINEAR@
                } else {
                    @HEAD_SHARED@
                };
                let mut chain = AnyChain::D0(head);
                let mut code = code;
                for _ in 0..len {
                    chain = chain.adapt(adapter(code % KINDS, &r));
                    code /= KINDS;
                }
                types.insert(chain.chain_type());
                chain.materialize(&mut b, which, &mut cells, &sink);
                which += 1;
            }
        }
    }
    for t in 0..10i64 {
        for i in &inputs {
            b.send(i, t * 7 + 1);
        }
        b.send(&shared_in, t * 3 + 2);
        b.run();
    }
    let checksum = cells
        .iter()
        .fold(sink.get(), |a, c| a.wrapping_mul(31).wrapping_add(*b.sample(*c)));
    println!("chain types: {}", types.len());
    println!("chains: {which}");
    println!("checksum: {checksum}");
}
"#;

/// The generated `main.rs` for one design, depth and number of kinds.
fn main_rs(design: Design, depth: usize, kinds: usize) -> String {
    let prefix = format!("{}::", design.bound);
    let variants: Vec<String> = KINDS[..kinds]
        .iter()
        .map(|k| format!("    {},", k.0))
        .collect();
    let makes: Vec<String> = KINDS[..kinds]
        .iter()
        .enumerate()
        .map(|(i, k)| format!("        {i} => {},", k.1))
        .collect();
    let arms: Vec<String> = KINDS[..kinds]
        .iter()
        .map(|k| format!("            {},", k.2.replace("P::", &prefix)))
        .collect();
    let mut chains = String::new();
    for d in 0..depth {
        chains += &format!(
            "trait Chain{d}: Materialize {{\n    fn adapt(self: Box<Self>, a: Adapter) -> Box<dyn Chain{n}>;\n}}\n\
             impl<S: {bound}<Event = i64>> Chain{d} for S {{\n    fn adapt(self: Box<Self>, a: Adapter) -> Box<dyn Chain{n}> {{\n        adapt!(*self, a, Box<dyn Chain{n}>)\n    }}\n}}\n\n",
            n = d + 1,
            bound = design.bound,
        );
    }
    chains += &format!(
        "trait Chain{depth}: Materialize {{}}\nimpl<S: {}<Event = i64>> Chain{depth} for S {{}}\n",
        design.bound
    );
    let any: Vec<String> = (0..=depth)
        .map(|d| format!("    D{d}(Box<dyn Chain{d}>),"))
        .collect();
    let mut any_adapt: Vec<String> = (0..depth)
        .map(|d| {
            format!(
                "            AnyChain::D{d}(c) => AnyChain::D{}(c.adapt(a)),",
                d + 1
            )
        })
        .collect();
    any_adapt.push(format!(
        "            AnyChain::D{depth}(_) => unreachable!(\"deeper than the bound\"),"
    ));
    let any_type: Vec<String> = (0..=depth)
        .map(|d| format!("            AnyChain::D{d}(c) => c.chain_type(),"))
        .collect();
    let any_materialize: Vec<String> = (0..=depth)
        .map(|d| format!("            AnyChain::D{d}(c) => materialize(c, b, which, cells, sink),"))
        .collect();
    MAIN.replace("@DESIGN@", design.name)
        .replace("@DEPTH@", &depth.to_string())
        .replace("@KINDS@", &kinds.to_string())
        .replace("@MATERIALIZERS@", design.materializers)
        .replace("@BOUND@", design.bound)
        .replace("@VARIANTS@", &variants.join("\n"))
        .replace("@MAKES@", &makes.join("\n"))
        .replace("@ARMS@", &arms.join("\n"))
        .replace("@CHAINS@", &chains)
        .replace("@ANY@", &any.join("\n"))
        .replace("@ANY_ADAPT@", &any_adapt.join("\n"))
        .replace("@ANY_TYPE@", &any_type.join("\n"))
        .replace("@ANY_MATERIALIZE@", &any_materialize.join("\n"))
        .replace("@HEAD_LINEAR@", &design.head.replace("HEAD", "s"))
        .replace("@HEAD_SHARED@", &design.head.replace("HEAD", "shared"))
}

struct Crate {
    design: Design,
    depth: usize,
    dir: PathBuf,
    name: String,
    times: Vec<f64>,
}

impl Crate {
    fn generate(root: &Path, design: Design, depth: usize, kinds: usize) -> Crate {
        let name = format!("rfd0004-{}-d{depth}", design.name);
        let dir = root.join(&name);
        std::fs::create_dir_all(dir.join("src")).expect("create the generated crate");
        // Its own workspace, so the experiments workspace doesn't claim it.
        let manifest = format!(
            "[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n[workspace]\n"
        );
        std::fs::write(dir.join("Cargo.toml"), manifest).expect("write Cargo.toml");
        std::fs::write(dir.join("src/engine.rs"), ENGINE).expect("write engine.rs");
        std::fs::write(dir.join("src/main.rs"), main_rs(design, depth, kinds))
            .expect("write main.rs");
        Crate {
            design,
            depth,
            dir,
            name,
            times: Vec::new(),
        }
    }

    fn target(&self) -> PathBuf {
        self.dir.join("target")
    }

    fn binary(&self) -> PathBuf {
        self.target().join("release").join(&self.name)
    }

    /// One release build from clean, in seconds.
    fn build(&mut self) -> f64 {
        let _ = std::fs::remove_dir_all(self.target());
        let start = Instant::now();
        let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
            .args(["build", "--release", "--offline", "--quiet", "--target-dir"])
            .arg(self.target())
            .current_dir(&self.dir)
            .env_remove("CARGO_TARGET_DIR")
            .status()
            .expect("run cargo");
        let secs = start.elapsed().as_secs_f64();
        assert!(status.success(), "{} did not build", self.name);
        self.times.push(secs);
        secs
    }

    fn median(&self) -> f64 {
        let mut t = self.times.clone();
        t.sort_by(f64::total_cmp);
        let n = t.len();
        if n % 2 == 1 {
            t[n / 2]
        } else {
            (t[n / 2 - 1] + t[n / 2]) / 2.0
        }
    }

    /// The generated program's own report: chain types and checksum.
    fn run(&self) -> (String, String) {
        let out = Command::new(self.binary())
            .output()
            .expect("run the built program");
        assert!(out.status.success(), "{} failed", self.name);
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let field = |key: &str| {
            text.lines()
                .find_map(|l| l.strip_prefix(key))
                .unwrap_or("?")
                .trim()
                .to_string()
        };
        (field("chain types:"), field("checksum:"))
    }

    /// Node eval functions in the binary: one per node type the compiler
    /// instantiated, since each is reached through an ops table and can't be
    /// inlined away.
    fn eval_fns(&self) -> String {
        let Ok(out) = Command::new("nm").arg("-C").arg(self.binary()).output() else {
            return "n/a".into();
        };
        let text = String::from_utf8_lossy(&out.stdout);
        text.lines()
            .filter(|l| l.contains("engine::eval_"))
            .count()
            .to_string()
    }

    fn size_kib(&self) -> u64 {
        std::fs::metadata(self.binary())
            .map(|m| m.len() / 1024)
            .unwrap_or(0)
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let quick = args.iter().any(|a| a == "--quick");
    let (kinds, depths, mut reps) = if quick {
        (5, vec![2], 1)
    } else {
        (9, vec![2, 3], 3)
    };
    if let Some(i) = args.iter().position(|a| a == "--reps") {
        reps = args
            .get(i + 1)
            .and_then(|n| n.parse().ok())
            .expect("--reps takes a number");
    }

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/rfd-0004-erased-materializer");
    let mut crates: Vec<Crate> = depths
        .iter()
        .flat_map(|&depth| DESIGNS.map(|d| (d, depth)))
        .map(|(d, depth)| Crate::generate(&root, d, depth, kinds))
        .collect();

    println!(
        "F36's chain shapes, from data: {kinds} adapter kinds ({}), a linear or a shared head,",
        KIND_NAMES[..kinds].join(", ")
    );
    println!(
        "six materializers, every chain of up to {} adapters; generated under {}.",
        depths
            .iter()
            .map(|d| d.to_string())
            .collect::<Vec<_>>()
            .join(" and "),
        root.display()
    );
    println!(
        "Release build of each crate from clean, {reps} time{}, interleaved.{}",
        if reps == 1 { "" } else { "s" },
        if quick {
            " Quick mode: indicative only."
        } else {
            ""
        }
    );
    println!();

    let started = Instant::now();
    for rep in 0..reps {
        for c in crates.iter_mut() {
            let secs = c.build();
            eprintln!(
                "rep {}/{reps}: {:<8} depth {}: {secs:.1} s",
                rep + 1,
                c.design.name,
                c.depth
            );
        }
    }
    let total = started.elapsed().as_secs_f64();

    println!(
        "{:<6} {:<9} {:>11} {:>10} {:>10} {:>11}  {:<24} {:>11}  checksum",
        "depth",
        "design",
        "chain types",
        "eval fns",
        "median s",
        "vs baseline",
        "builds (s)",
        "binary KiB"
    );
    let mut verdict = Vec::new();
    for &depth in &depths {
        let here: Vec<&Crate> = crates.iter().filter(|c| c.depth == depth).collect();
        let base = here[0].median();
        let mut sums = Vec::new();
        for c in &here {
            let (types, checksum) = c.run();
            sums.push(checksum.clone());
            let runs: Vec<String> = c.times.iter().map(|t| format!("{t:.1}")).collect();
            println!(
                "{:<6} {:<9} {:>11} {:>10} {:>10.1} {:>11.2}  {:<24} {:>11}  {checksum}",
                depth,
                c.design.name,
                types,
                c.eval_fns(),
                c.median(),
                c.median() / base,
                runs.join(" "),
                c.size_kib(),
            );
        }
        let agree = sums.iter().all(|s| *s == sums[0]);
        verdict.push(format!(
            "at depth {depth} boxed built in {:.2}, fnptr in {:.2} and flat in {:.2} of baseline's time{}",
            here[1].median() / base,
            here[2].median() / base,
            here[3].median() / base,
            if agree {
                ", every checksum the same"
            } else {
                ", and the checksums DIFFER"
            }
        ));
    }
    println!();
    println!("builds took {total:.0} s in all");
    println!("verdict: {}.", verdict.join("; "));
}
