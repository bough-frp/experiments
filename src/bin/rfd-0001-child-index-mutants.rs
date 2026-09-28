//! Does the oracle's comparison catch an engine that puts an event in the
//! wrong sibling child instant, and are the ones it misses observable at
//! all?
//!
//! RFD 1's test affordances can't show which child instant an event fell
//! in, so the spike's comparison checks, per observed node, the order of
//! events within each top-level transaction; the order of listener calls
//! across nodes, read against the oracle's times; and a sample of every
//! observed cell after each top-level transaction. Child indices are not
//! compared (spike note, "Where the RFDs are wrong / RFD 1").
//!
//! The probe is a toy interpreter of RFD 5's hierarchical time, `T =
//! [Int]` ordered lexicographically, so `[t] < [t, 0] < [t, 1] < [t + 1]`.
//! `split` puts its i-th element at `t ++ [i]`, `defer` at `t ++ [0]`, and
//! every split and defer at t shares t's child indices. The nodes are
//! `input`, `map`, `filter`, `merge` (left-biased, or Sodium's combining
//! function), `hold`, `snapshot` (reads the cell before the instant),
//! `split` and `defer`, in random acyclic programs over random inputs.
//!
//! A mutant is the reference run with one split's or defer's output moved
//! to sibling child instants, and everything downstream recomputed:
//!
//! - `later`, `earlier`: one event from `t ++ [i]` to `t ++ [i ± 1]`, even
//!   onto a sibling of the same node.
//! - `offset`: one split's or defer's children at t numbered after the
//!   other splits' and defers' children at t, as if indices weren't shared.
//! - `collapse`: all of one split's children at t at `t ++ [0]` (F2).
//! - `defer-at-1`: every event of one defer at `t ++ [1]`.
//!
//! Each mutant is held to the reference two ways: `without`, what the
//! oracle checks now, and `with`, the same plus every observed event's
//! full time. Each survivor of `without` is then classed by what could see
//! it: (A) the same comparison with every node observed; (B) a change in
//! the order or simultaneity of two events anywhere in the run, which a
//! merge, or a snapshot of a hold, over the two nodes would see; (C)
//! neither: the run is order-isomorphic to the reference, so no node of
//! this program, and no merge, snapshot or hold added over them, can tell.
//! Only a new split or defer at the same parent instant could, by
//! colliding with a moved index.
//!
//! A survivor harmless in one program may come from an engine bug another
//! program shows, so three engine-wide bugs, applied to every split or
//! defer at once, are judged per program the same way: every defer at
//! index 1, indices not shared between splits, and F2's collapse. A suite
//! catches such a bug if any one of its programs does.

use std::collections::HashMap;
use std::fmt::Write as _;

type Time = Vec<u32>;
/// A stream's events, or a cell's steps, sorted by time, stably. The
/// reference never has two at one time in one node; mutants may.
type Trace = Vec<(Time, i64)>;

const MODULUS: i64 = 1_000_003;
const TRANSACTIONS: u32 = 6;
const INPUTS: usize = 2;
/// Mutants drawn per operator per program, so large programs don't
/// dominate.
const PER_OPERATOR: usize = 8;

/// splitmix64: small, seedable, good enough to draw programs.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

#[derive(Clone, Copy, Debug)]
enum Def {
    Input(usize),
    Map {
        s: usize,
        a: i64,
        b: i64,
    },
    /// Keeps values whose residue mod 3 isn't `r`.
    Filter {
        s: usize,
        r: i64,
    },
    Merge {
        l: usize,
        r: usize,
        combine: bool,
    },
    Hold {
        initial: i64,
        s: usize,
    },
    Snapshot {
        s: usize,
        c: usize,
    },
    /// Splits v into `v mod 4` elements, zero to three.
    Split {
        s: usize,
    },
    Defer {
        s: usize,
    },
}

impl Def {
    fn is_cell(self) -> bool {
        matches!(self, Def::Hold { .. })
    }
}

struct Program {
    defs: Vec<Def>,
    observed: Vec<usize>,
    /// Each input's sends, at top-level instants `[k]`.
    inputs: Vec<Trace>,
}

fn combine(a: i64, b: i64) -> i64 {
    (a + 2 * b) % MODULUS
}

fn element(v: i64, i: u32) -> i64 {
    (v * 31 + i64::from(i) * 17 + 5) % MODULUS
}

fn child(t: &Time, i: u32) -> Time {
    let mut c = t.clone();
    c.push(i);
    c
}

/// A cell's value read at t: its last step strictly before t.
fn before(initial: i64, steps: &Trace, t: &Time) -> i64 {
    let n = steps.partition_point(|(s, _)| s < t);
    if n == 0 { initial } else { steps[n - 1].1 }
}

/// A cell's value after top-level transaction k and its children.
fn sample(initial: i64, steps: &Trace, k: u32) -> i64 {
    let n = steps.partition_point(|(s, _)| s[0] <= k);
    if n == 0 { initial } else { steps[n - 1].1 }
}

/// Merge instant by instant. Simultaneous events on one side (only a
/// mutant has them) fold with the combining function, or all pass through.
fn merge(l: &Trace, r: &Trace, with: bool) -> Trace {
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    loop {
        let t = match (l.get(i), r.get(j)) {
            (None, None) => break,
            (Some(a), None) => a.0.clone(),
            (None, Some(b)) => b.0.clone(),
            (Some(a), Some(b)) => a.0.clone().min(b.0.clone()),
        };
        let i2 = i + l[i..].iter().take_while(|e| e.0 == t).count();
        let j2 = j + r[j..].iter().take_while(|e| e.0 == t).count();
        let (left, right) = (&l[i..i2], &r[j..j2]);
        let fold = |es: &[(Time, i64)]| es.iter().map(|e| e.1).reduce(combine).unwrap();
        if with && !left.is_empty() && !right.is_empty() {
            out.push((t, combine(fold(left), fold(right))));
        } else if !left.is_empty() {
            out.extend_from_slice(left);
        } else {
            out.extend_from_slice(right);
        }
        (i, j) = (i2, j2);
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operator {
    Later,
    Earlier,
    Offset,
    Collapse,
    DeferAt1,
}

const OPERATORS: [Operator; 5] = [
    Operator::Later,
    Operator::Earlier,
    Operator::Offset,
    Operator::Collapse,
    Operator::DeferAt1,
];

impl Operator {
    fn name(self) -> &'static str {
        match self {
            Operator::Later => "later",
            Operator::Earlier => "earlier",
            Operator::Offset => "offset",
            Operator::Collapse => "collapse",
            Operator::DeferAt1 => "defer-at-1",
        }
    }
}

#[derive(Clone, Debug)]
struct Mutation {
    node: usize,
    operator: Operator,
    /// The instant whose children move; unused by `defer-at-1`.
    parent: Time,
    /// The child moved by `later` and `earlier`.
    index: u32,
    /// How far `offset` moves.
    by: u32,
}

impl Mutation {
    fn apply(&self, trace: &mut Trace) {
        for (t, _) in trace.iter_mut() {
            let last = t.len() - 1;
            let here = t[..last] == self.parent[..];
            match self.operator {
                Operator::Later if here && t[last] == self.index => t[last] += 1,
                Operator::Earlier if here && t[last] == self.index => t[last] -= 1,
                Operator::Offset if here => t[last] += self.by,
                Operator::Collapse if here => t[last] = 0,
                Operator::DeferAt1 => t[last] = 1,
                _ => {}
            }
        }
        // Stable: an event moved onto a sibling keeps its place beside it,
        // so the node's own order of values never changes.
        trace.sort_by(|a, b| a.0.cmp(&b.0));
    }
}

/// An engine-wide bug, as against a [`Mutation`] of one site: it applies
/// to every split or defer of a program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Bug {
    /// Every defer at `t ++ [1]`.
    DeferAt1,
    /// Each split or defer numbers its children at t after those of the
    /// splits and defers defined before it: indices not shared.
    Unshared,
    /// Every split puts all its children at t at `t ++ [0]` (F2).
    CollapseAll,
}

const BUGS: [Bug; 3] = [Bug::DeferAt1, Bug::Unshared, Bug::CollapseAll];

impl Bug {
    fn name(self) -> &'static str {
        match self {
            Bug::DeferAt1 => "every defer at index 1",
            Bug::Unshared => "indices not shared",
            Bug::CollapseAll => "every split collapsed (F2)",
        }
    }
}

/// Every node's trace, the mutation applied to its node's output, or the
/// bug to every split's and defer's. Nodes upstream of the mutated one are
/// the reference's, so a mutation can name its events by their reference
/// times.
fn run(p: &Program, mutation: Option<&Mutation>, bug: Option<Bug>) -> Vec<Trace> {
    let mut out: Vec<Trace> = Vec::with_capacity(p.defs.len());
    // For `Unshared`: the next free child index at each parent.
    let mut next: HashMap<Time, u32> = HashMap::new();
    for (n, def) in p.defs.iter().enumerate() {
        let mut trace: Trace = match *def {
            Def::Input(i) => p.inputs[i].clone(),
            Def::Map { s, a, b } => out[s]
                .iter()
                .map(|(t, v)| (t.clone(), (v * a + b) % MODULUS))
                .collect(),
            Def::Filter { s, r } => out[s].iter().filter(|e| e.1 % 3 != r).cloned().collect(),
            Def::Merge { l, r, combine } => merge(&out[l], &out[r], combine),
            Def::Hold { s, .. } => out[s].clone(),
            Def::Snapshot { s, c } => {
                let Def::Hold { initial, .. } = p.defs[c] else {
                    unreachable!("a snapshot reads a hold")
                };
                out[s]
                    .iter()
                    .map(|(t, v)| (t.clone(), (3 * v + before(initial, &out[c], t)) % MODULUS))
                    .collect()
            }
            Def::Split { s } => {
                let mut children: Trace = out[s]
                    .iter()
                    .flat_map(|(t, v)| (0..(v % 4) as u32).map(|i| (child(t, i), element(*v, i))))
                    .collect();
                children.sort_by(|a, b| a.0.cmp(&b.0));
                children
            }
            Def::Defer { s } => out[s].iter().map(|(t, v)| (child(t, 0), *v)).collect(),
        };
        if let Some(m) = mutation.filter(|m| m.node == n) {
            m.apply(&mut trace);
        }
        let generator = matches!(def, Def::Split { .. } | Def::Defer { .. });
        if let Some(bug) = bug.filter(|_| generator) {
            let mut used: HashMap<Time, u32> = HashMap::new();
            for (t, _) in trace.iter_mut() {
                let last = t.len() - 1;
                match bug {
                    Bug::DeferAt1 if matches!(def, Def::Defer { .. }) => t[last] = 1,
                    Bug::CollapseAll if matches!(def, Def::Split { .. }) => t[last] = 0,
                    Bug::Unshared => {
                        let at = parent(t);
                        let high = used.entry(at.clone()).or_insert(0);
                        *high = (*high).max(t[last] + 1);
                        t[last] += next.get(&at).copied().unwrap_or(0);
                    }
                    _ => {}
                }
            }
            for (at, high) in used {
                *next.entry(at).or_insert(0) += high;
            }
            trace.sort_by(|a, b| a.0.cmp(&b.0));
        }
        out.push(trace);
    }
    out
}

// ------------------------------------------------------------ generation

/// An operand of the given kind; half the time one of the last few, so
/// chains get deep.
fn pick(rng: &mut Rng, defs: &[Def], cell: bool) -> Option<usize> {
    let fit: Vec<usize> = (0..defs.len())
        .filter(|&n| defs[n].is_cell() == cell)
        .collect();
    if fit.is_empty() {
        None
    } else if rng.chance(50) {
        Some(fit[fit.len() - 1 - rng.below(fit.len().min(4))])
    } else {
        Some(fit[rng.below(fit.len())])
    }
}

fn generate(rng: &mut Rng) -> Program {
    let mut defs: Vec<Def> = (0..INPUTS).map(Def::Input).collect();
    let extra = 6 + rng.below(11);
    for _ in 0..extra {
        let s = pick(rng, &defs, false).unwrap();
        let def = match rng.below(16) {
            0 | 1 => Def::Map {
                s,
                a: 1 + rng.below(9) as i64,
                b: rng.below(100) as i64,
            },
            2 => Def::Filter {
                s,
                r: rng.below(3) as i64,
            },
            3..=5 => Def::Merge {
                l: s,
                r: pick(rng, &defs, false).unwrap(),
                combine: rng.chance(50),
            },
            6..=8 => Def::Hold {
                initial: rng.below(100) as i64,
                s,
            },
            9..=11 => match pick(rng, &defs, true) {
                Some(c) => Def::Snapshot { s, c },
                None => Def::Hold { initial: 0, s },
            },
            12 | 13 => Def::Split { s },
            _ => Def::Defer { s },
        };
        defs.push(def);
    }
    let mut observed: Vec<usize> = (INPUTS..defs.len()).filter(|_| rng.chance(40)).collect();
    if observed.is_empty() {
        observed.push(defs.len() - 1);
    }
    let mut inputs = vec![Trace::new(); INPUTS];
    for sends in &mut inputs {
        for k in 1..=TRANSACTIONS {
            if rng.chance(60) {
                sends.push((vec![k], rng.below(1000) as i64));
            }
        }
    }
    Program {
        defs,
        observed,
        inputs,
    }
}

fn parent(t: &Time) -> Time {
    t[..t.len() - 1].to_vec()
}

/// Every mutant the reference run offers, as the operators define them.
fn sites(p: &Program, reference: &[Trace]) -> Vec<Mutation> {
    let mut sites = Vec::new();
    let generators: Vec<usize> = (0..p.defs.len())
        .filter(|&n| matches!(p.defs[n], Def::Split { .. } | Def::Defer { .. }))
        .collect();
    for &n in &generators {
        let mutation = |operator, parent: Time, index, by| Mutation {
            node: n,
            operator,
            parent,
            index,
            by,
        };
        let mut parents: Vec<Time> = reference[n].iter().map(|e| parent(&e.0)).collect();
        parents.dedup();
        for (t, _) in &reference[n] {
            let i = *t.last().unwrap();
            sites.push(mutation(Operator::Later, parent(t), i, 0));
            if i > 0 {
                sites.push(mutation(Operator::Earlier, parent(t), i, 0));
            }
        }
        for at in &parents {
            // The highest index another split or defer used at this parent.
            let others = generators
                .iter()
                .filter(|&&m| m != n)
                .flat_map(|&m| &reference[m])
                .filter(|(t, _)| parent(t) == *at)
                .map(|(t, _)| *t.last().unwrap())
                .max();
            if let Some(high) = others {
                sites.push(mutation(Operator::Offset, at.clone(), 0, high + 1));
            }
            let mine = reference[n].iter().filter(|e| parent(&e.0) == *at).count();
            if matches!(p.defs[n], Def::Split { .. }) && mine >= 2 {
                sites.push(mutation(Operator::Collapse, at.clone(), 0, 0));
            }
        }
        if matches!(p.defs[n], Def::Defer { .. }) && !reference[n].is_empty() {
            sites.push(mutation(Operator::DeferAt1, Vec::new(), 0, 0));
        }
    }
    sites
}

// ------------------------------------------------------------ comparison

fn in_transaction(trace: &Trace, k: u32) -> &[(Time, i64)] {
    let a = trace.partition_point(|e| e.0[0] < k);
    let b = trace.partition_point(|e| e.0[0] <= k);
    &trace[a..b]
}

/// The oracle's per-node check: each top-level transaction's values in
/// order, and for a cell its sample after the transaction.
fn node_differs(def: Def, r: &Trace, m: &Trace) -> bool {
    (1..=TRANSACTIONS).any(|k| {
        let values = |t: &Trace| in_transaction(t, k).iter().map(|e| e.1).collect::<Vec<_>>();
        values(r) != values(m)
            || matches!(def, Def::Hold { initial, .. }
                if sample(initial, r, k) != sample(initial, m, k))
    })
}

/// Every event of these nodes in transaction k, as (node, reference time,
/// mutant time), paired by position, which is sound once the per-node
/// lists agree.
fn paired<'a>(
    nodes: &[usize],
    r: &'a [Trace],
    m: &'a [Trace],
    k: u32,
) -> Vec<(usize, &'a Time, &'a Time)> {
    nodes
        .iter()
        .flat_map(|&n| {
            let (a, b) = (in_transaction(&r[n], k), in_transaction(&m[n], k));
            a.iter().zip(b).map(move |(x, y)| (n, &x.0, &y.0))
        })
        .collect()
}

/// The oracle's order check: a listener call that must come after a call
/// for a later reference time. Calls at one mutant time may run in any
/// order, so only a strict inversion is certain to be caught.
fn order_inverted(nodes: &[usize], r: &[Trace], m: &[Trace]) -> bool {
    (1..=TRANSACTIONS).any(|k| {
        let calls = paired(nodes, r, m, k);
        calls
            .iter()
            .any(|(_, r1, m1)| calls.iter().any(|(_, r2, m2)| m1 < m2 && r1 > r2))
    })
}

fn caught_without(p: &Program, nodes: &[usize], r: &[Trace], m: &[Trace]) -> bool {
    nodes.iter().any(|&n| node_differs(p.defs[n], &r[n], &m[n])) || order_inverted(nodes, r, m)
}

fn caught_with(p: &Program, nodes: &[usize], r: &[Trace], m: &[Trace]) -> bool {
    caught_without(p, nodes, r, m) || nodes.iter().any(|&n| r[n] != m[n])
}

type Event = (usize, Time, Time);

/// Two events anywhere in the run whose order or simultaneity the mutant
/// changed, a pair across two nodes if there is one, else a pair within
/// one. Only asked once every node's lists agree, so pairing by position
/// is sound.
fn preorder_pair(p: &Program, r: &[Trace], m: &[Trace]) -> Option<(Event, Event)> {
    let all: Vec<usize> = (0..p.defs.len()).collect();
    let mut within = None;
    for k in 1..=TRANSACTIONS {
        let events = paired(&all, r, m, k);
        for x in &events {
            for y in &events {
                if x.1.cmp(y.1) != x.2.cmp(y.2) {
                    let own = |e: &(usize, &Time, &Time)| (e.0, e.1.clone(), e.2.clone());
                    if x.0 != y.0 {
                        return Some((own(x), own(y)));
                    }
                    within.get_or_insert((own(x), own(y)));
                }
            }
        }
    }
    within
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Shape {
    /// No merge or snapshot takes an affected node.
    Alone,
    /// One does, and its other input is quiet in the affected transactions.
    Quiet,
    /// One does, and its other input fires or steps in them.
    Active,
}

const SHAPES: [Shape; 3] = [Shape::Alone, Shape::Quiet, Shape::Active];

impl Shape {
    fn name(self) -> &'static str {
        match self {
            Shape::Alone => "no merge/snapshot downstream",
            Shape::Quiet => "merge/snapshot, other side quiet",
            Shape::Active => "merge/snapshot, other side active",
        }
    }

    fn short(self) -> &'static str {
        match self {
            Shape::Alone => "alone",
            Shape::Quiet => "quiet",
            Shape::Active => "active",
        }
    }
}

/// Whether a merge or snapshot takes a node the mutant changed, and if so
/// whether its other input is busy in a transaction where that changed.
fn shape(p: &Program, r: &[Trace], m: &[Trace]) -> Shape {
    let affected: Vec<bool> = r.iter().zip(m).map(|(a, b)| a != b).collect();
    let mut shape = Shape::Alone;
    for def in &p.defs {
        let (a, b) = match *def {
            Def::Merge { l, r, .. } => (l, r),
            Def::Snapshot { s, c } => (s, c),
            _ => continue,
        };
        for (x, other) in [(a, b), (b, a)] {
            if !affected[x] {
                continue;
            }
            let active = (1..=TRANSACTIONS).any(|k| {
                in_transaction(&r[x], k) != in_transaction(&m[x], k)
                    && !in_transaction(&r[other], k).is_empty()
            });
            shape = shape.max(if active { Shape::Active } else { Shape::Quiet });
        }
    }
    shape
}

// ------------------------------------------------------------ tally

/// What could see a survivor of `without`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    /// The same comparison, with every node observed.
    A,
    /// A merge or snapshot added over two nodes whose events became, or
    /// stopped being, simultaneous.
    Across,
    /// A merge or snapshot added over one node two of whose own events
    /// became, or stopped being, simultaneous.
    Within,
    /// Nothing built from the program's nodes: order-isomorphic.
    C,
}

const CLASSES: [Class; 4] = [Class::A, Class::Across, Class::Within, Class::C];

impl Class {
    fn name(self) -> &'static str {
        match self {
            Class::A => "A",
            Class::Across => "B across nodes",
            Class::Within => "B within a node",
            Class::C => "C",
        }
    }
}

#[derive(Default, Clone, Copy)]
struct Tally {
    mutants: u32,
    without: u32,
    /// Caught by `without` through the listener order alone.
    order_only: u32,
    with: u32,
    /// Survivors of `without`, by class.
    survivors: [u32; 4],
    /// Survivors of `with`, by class.
    with_survivors: [u32; 4],
    /// Mutants `with` misses even with every node observed: should be none,
    /// since every operator moves at least one event.
    with_all_missed: u32,
}

struct Outcome {
    without: bool,
    order_only: bool,
    with: bool,
    with_all: bool,
    class: Class,
}

impl Tally {
    fn add(&mut self, o: &Outcome) {
        let class = CLASSES.iter().position(|&c| c == o.class).unwrap();
        self.mutants += 1;
        self.without += u32::from(o.without);
        self.order_only += u32::from(o.order_only);
        self.with += u32::from(o.with);
        if !o.without {
            self.survivors[class] += 1;
        }
        if !o.with {
            self.with_survivors[class] += 1;
        }
        self.with_all_missed += u32::from(!o.with_all);
    }

    fn row(&self, label: &str) -> String {
        let pct = |x: u32| 100.0 * f64::from(x) / f64::from(self.mutants.max(1));
        let [a, across, within, c] = self.survivors;
        format!(
            "{label:<34} {:>6} {:>7.1}% {:>6.1}% {:>6.1}% | {:>5} {:>6} {:>6} {:>5} | {:>6.1}%\n",
            self.mutants,
            pct(self.without),
            pct(self.order_only),
            pct(self.with),
            a,
            across,
            within,
            c,
            pct(self.mutants - across - within - c),
        )
    }
}

const HEADER: &str = "                                  mutants without  order    with |     A B-acr. B-with     C | all-obs\n";

struct Example {
    size: usize,
    text: String,
}

fn show(p: &Program, m: &Mutation, r: &[Trace], mt: &[Trace], note: &str) -> String {
    let mut s = String::new();
    for (n, def) in p.defs.iter().enumerate() {
        let seen = if p.observed.contains(&n) {
            "  (observed)"
        } else {
            ""
        };
        let _ = writeln!(s, "    n{n} = {def:?}{seen}");
    }
    let _ = writeln!(s, "    inputs: {:?}", p.inputs);
    let _ = writeln!(
        s,
        "    mutation: {} on n{}, parent {:?}",
        m.operator.name(),
        m.node,
        m.parent
    );
    let _ = writeln!(s, "    n{} reference {:?}", m.node, r[m.node]);
    let _ = writeln!(s, "    n{} mutant    {:?}", m.node, mt[m.node]);
    let _ = writeln!(s, "    {note}");
    s
}

/// Every check on one mutant run, and for a survivor of every node
/// observed, the pair of events that shows it isn't order-isomorphic.
fn judge(p: &Program, reference: &[Trace], mutant: &[Trace]) -> (Outcome, Option<(Event, Event)>) {
    let all: Vec<usize> = (0..p.defs.len()).collect();
    let values = p
        .observed
        .iter()
        .any(|&n| node_differs(p.defs[n], &reference[n], &mutant[n]));
    let without = caught_without(p, &p.observed, reference, mutant);
    let caught_all = caught_without(p, &all, reference, mutant);
    let pair = if caught_all {
        None
    } else {
        preorder_pair(p, reference, mutant)
    };
    // What survives every node observed can only be a change of
    // simultaneity: a strict reversal fails the listener order.
    if let Some((x, y)) = &pair {
        assert!(x.1 == y.1 || x.2 == y.2, "a reversal survived: {x:?} {y:?}");
    }
    let class = match &pair {
        _ if caught_all => Class::A,
        Some((x, y)) if x.0 != y.0 => Class::Across,
        Some(_) => Class::Within,
        None => Class::C,
    };
    let outcome = Outcome {
        without,
        order_only: without && !values,
        with: caught_with(p, &p.observed, reference, mutant),
        with_all: caught_with(p, &all, reference, mutant),
        class,
    };
    (outcome, pair)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let programs: u32 = args.next().map_or(4000, |a| a.parse().expect("programs"));
    let seed: u64 = args.next().map_or(20260928, |a| a.parse().expect("seed"));
    let mut rng = Rng(seed);

    let mut by_operator = [Tally::default(); 5];
    let mut by_shape = [Tally::default(); 3];
    let mut grid = [[Tally::default(); 3]; 5];
    let mut total = Tally::default();
    let mut examples: [Option<Example>; 4] = [None, None, None, None];
    let mut with_sites = 0;
    // Per engine-wide bug: programs it changed anything in, and their tally.
    let mut bugs = [Tally::default(); 3];

    for _ in 0..programs {
        let p = generate(&mut rng);
        let reference = run(&p, None, None);
        for (b, bug) in BUGS.iter().enumerate() {
            let mutant = run(&p, None, Some(*bug));
            if mutant != reference {
                bugs[b].add(&judge(&p, &reference, &mutant).0);
            }
        }
        let mut sites = sites(&p, &reference);
        with_sites += u32::from(!sites.is_empty());
        // Draw up to PER_OPERATOR of each operator, in a shuffled order.
        for i in (1..sites.len()).rev() {
            sites.swap(i, rng.below(i + 1));
        }
        let mut drawn = [0; 5];
        for m in sites {
            let o = OPERATORS.iter().position(|&x| x == m.operator).unwrap();
            if drawn[o] == PER_OPERATOR {
                continue;
            }
            drawn[o] += 1;
            let mutant = run(&p, Some(&m), None);
            let (outcome, pair) = judge(&p, &reference, &mutant);
            let (without, class) = (outcome.without, outcome.class);
            let s = SHAPES
                .iter()
                .position(|&x| x == shape(&p, &reference, &mutant))
                .unwrap();
            by_operator[o].add(&outcome);
            by_shape[s].add(&outcome);
            grid[o][s].add(&outcome);
            total.add(&outcome);
            let c = CLASSES.iter().position(|&x| x == class).unwrap();
            if !without && examples[c].as_ref().is_none_or(|e| p.defs.len() < e.size) {
                let note = match &pair {
                    Some((x, y)) => format!(
                        "order changed: n{} {:?} -> {:?} against n{} {:?} -> {:?}",
                        x.0, x.1, x.2, y.0, y.1, y.2
                    ),
                    None if class == Class::A => "caught once every node is observed".to_owned(),
                    None => "every pair of events keeps its order".to_owned(),
                };
                examples[c] = Some(Example {
                    size: p.defs.len(),
                    text: show(&p, &m, &reference, &mutant, &note),
                });
            }
        }
    }

    let mut out = String::new();
    let _ = writeln!(
        out,
        "child-index-mutants: sibling child-instant mutants against the oracle's comparison\n"
    );
    let _ = writeln!(
        out,
        "{programs} programs (seed {seed}), {with_sites} with a split or defer that fired; \
         {TRANSACTIONS} transactions, {INPUTS} inputs, 8-18 nodes, about 40% observed;\n\
         up to {PER_OPERATOR} mutants per operator per program.\n"
    );
    out.push_str(
        "without = per-node values per transaction, listener order, cell samples (the oracle now)\n\
         order   = caught by `without` through the listener order alone\n\
         with    = `without`, plus every observed event's full time\n\
         A..C    = survivors of `without`, by what could see them:\n\
         \x20 A       `without` with every node of the program observed\n\
         \x20 B-acr.  nothing in the program: every node's values, samples and listener order\n\
         \x20         agree; two nodes' events became or stopped being simultaneous, which a\n\
         \x20         merge, or a snapshot of a hold, added over the two would see\n\
         \x20 B-with  the same, two events of one node\n\
         \x20 C       nothing in the program, nor any merge, snapshot or hold added over its\n\
         \x20         nodes: the run is order-isomorphic; only an added split or defer at the\n\
         \x20         same parent could collide with the moved index\n\
         all-obs = caught by `without` with every node observed\n\n",
    );
    out.push_str("By operator\n");
    out.push_str(HEADER);
    for (o, op) in OPERATORS.iter().enumerate() {
        out.push_str(&by_operator[o].row(op.name()));
    }
    out.push_str(&total.row("all"));
    out.push_str("\nBy shape: does a merge or snapshot take a node the mutant changed?\n");
    out.push_str(HEADER);
    for (s, sh) in SHAPES.iter().enumerate() {
        out.push_str(&by_shape[s].row(sh.name()));
    }
    out.push_str("\nBy operator and shape\n");
    out.push_str(HEADER);
    for (o, op) in OPERATORS.iter().enumerate() {
        for (s, sh) in SHAPES.iter().enumerate() {
            let label = format!("{} / {}", op.name(), sh.short());
            out.push_str(&grid[o][s].row(&label));
        }
    }
    let _ = writeln!(
        out,
        "\nEngine-wide bugs: every split or defer of a program at once. Programs = those of the \
         {programs} the bug changed any\nevent's time in; a suite catches the bug if any one of \
         its programs does."
    );
    out.push_str(&HEADER.replacen("mutants", "program", 1));
    for (b, bug) in BUGS.iter().enumerate() {
        out.push_str(&bugs[b].row(bug.name()));
    }
    let w = total.with_survivors;
    let _ = writeln!(
        out,
        "\nSurvivors of `with`: {} (A {}, B across {}, B within {}, C {}); missed by `with` \
         with every node observed: {}.",
        w.iter().sum::<u32>(),
        w[0],
        w[1],
        w[2],
        w[3],
        total.with_all_missed
    );
    for (c, class) in CLASSES.iter().enumerate() {
        if let Some(e) = &examples[c] {
            let _ = writeln!(
                out,
                "\nSmallest survivor of `without` in class {}:\n{}",
                class.name(),
                e.text
            );
        }
    }
    let pct = |x: u32| 100.0 * f64::from(x) / f64::from(total.mutants.max(1));
    let [a, across, within, c] = total.survivors;
    let _ = writeln!(
        out,
        "Verdict: `without` caught {} of {} mutants ({:.1}%), `with` {} ({:.1}%). Of the {} \
         survivors of `without`, {a} ({:.1}% of mutants) are caught once every node is \
         observed; {} ({:.1}%) leave every node of the program unchanged and change only \
         simultaneity ({across} across nodes, {within} within one), which an added merge or \
         snapshot would see; {c} ({:.1}%) are order-isomorphic to the reference.",
        total.without,
        total.mutants,
        pct(total.without),
        total.with,
        pct(total.with),
        total.mutants - total.without,
        pct(a),
        across + within,
        pct(across + within),
        pct(c),
    );
    print!("{out}");
}
