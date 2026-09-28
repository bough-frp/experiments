//! How many of the same-instant cycles RFD 2's acyclicity rule refuses are
//! constructive, and do any fall outside the exclusive-gate pattern that
//! switching already expresses?
//!
//! RFD 2 keeps the dependency graph acyclic: every path around a loop
//! crosses a read of a cell from before the instant (or a child instant),
//! and a same-instant cycle is refused (F3). Esterel v5 and SCCharts accept
//! a cycle when it is *constructive*: evaluating every stream in the
//! three-valued logic {⊥, absent, present}, starting from ⊥ and applying
//! each node's must/cannot rule until nothing changes, settles every stream
//! in every state and for every input (Berry, pp. 83–84, 108). This probe
//! generates random stream programs over a toy subset, keeps those with at
//! least one same-instant cycle (so the rule refuses them), and decides
//! constructiveness by that Kleene iteration.
//!
//! The subset, each node one stream:
//!
//! - `map(f, a)`, `filter(p, a)`, `a.hold(0).steps()` (a hold's steps view
//!   doesn't delay, F3, so for presence and value it's `a`),
//! - `gate(c, a)` where `c` is `c0`, `c1`, `!c0` or `!c1`, a bool cell read
//!   from before the instant, so it is known when the instant starts and
//!   its edge is not a dependency,
//! - `merge(a, b, +)`, Sodium's merge with a combining function, and
//!   `a.or_else(b)`, the left-biased one.
//!
//! Values are integers mod 4. Choices the spec left open:
//!
//! - **Values are evaluated once presence is known.** A stream is ⊥,
//!   absent, present with its value still ⊥, or present with a value. A
//!   merge with one side present knows it's present before it knows its
//!   value. A filter's predicate on a ⊥ value is ⊥, so a filter's presence
//!   stays ⊥ until its input's value is known. Every rule is monotone, so
//!   the fixpoint is the least one and doesn't depend on evaluation order;
//!   the probe checks that by also sweeping in reverse.
//! - **Cells are free.** Constructiveness is checked for every valuation of
//!   `c0, c1`, not only reachable ones. Reachability could only add
//!   constructive programs whose gates are exclusive by an invariant, which
//!   a switch on the same cell expresses too.
//! - **Three input models**, since which instants exist decides the
//!   answer: `any` (every presence combination of the program's inputs,
//!   including none of them, which is what a subgraph sees when a
//!   transaction sends only to inputs elsewhere in the graph, and in a
//!   child instant of a `split` or `defer` anywhere), `>=1` (at
//!   least one of them fires: the program's inputs are the whole graph's)
//!   and `one` (exactly one fires per transaction). Each present input
//!   takes each of the four values.
//!
//! Each constructive program is then classified:
//!
//! - **(a) gates:** in every cell state, cutting the input edge of every
//!   closed gate leaves the graph acyclic. That is the exclusive-gate
//!   pattern: in each state a `switch_stream` selecting that state's
//!   acyclic wiring expresses it today.
//! - **(b1) dead branch:** not (a), but acyclic once each `or_else` whose
//!   left side is present in every instant of that state also has its
//!   right edge cut. The cycle runs through a branch that never matters.
//! - **(b2) per instant:** anything else. The cycle is live, and which
//!   edge breaks it changes from instant to instant.
//!
//! Separately, a (b) program is **value-dependent** if it stops being
//! constructive when each filter's outcome is a free boolean instead of
//! its predicate on the value.
//!
//! Under `any`, class (b) is empty by construction, and the census should
//! show it: every node here is present only if one of its inputs is, so in
//! the quiet instant nothing is present, and a node on a cycle with no
//! closed gate can never be the first on that cycle to become absent,
//! since each rule needs its cycle predecessor settled first (or both
//! sides, for the merges). So every open cycle stays ⊥, and constructive
//! means (a).

use std::fmt::Write as _;

const VALUES: u8 = 4;
const CELLS: usize = 2;
const SAMPLES: usize = 50_000;
const MAX_SIZE: usize = 8;

/// A stream's state within an instant, ordered ⊥ < absent and
/// ⊥ < present-with-⊥-value < present(v).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum V {
    Bot,
    Absent,
    PresentBot,
    Present(u8),
}

impl V {
    fn present(self) -> bool {
        matches!(self, V::PresentBot | V::Present(_))
    }
    fn settled(self) -> bool {
        matches!(self, V::Absent | V::Present(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Src {
    Input(usize),
    Node(usize),
}

#[derive(Clone, Copy, Debug)]
enum MapFn {
    Inc,
    Double,
    Add3,
}

impl MapFn {
    fn apply(self, v: u8) -> u8 {
        match self {
            MapFn::Inc => (v + 1) % VALUES,
            MapFn::Double => (v * 2) % VALUES,
            MapFn::Add3 => (v + 3) % VALUES,
        }
    }
    fn text(self) -> &'static str {
        match self {
            MapFn::Inc => "|v| v + 1",
            MapFn::Double => "|v| v * 2",
            MapFn::Add3 => "|v| v + 3",
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Pred {
    Even,
    Odd,
    Zero,
    NonZero,
}

impl Pred {
    fn test(self, v: u8) -> bool {
        match self {
            Pred::Even => v.is_multiple_of(2),
            Pred::Odd => !v.is_multiple_of(2),
            Pred::Zero => v == 0,
            Pred::NonZero => v != 0,
        }
    }
    fn text(self) -> &'static str {
        match self {
            Pred::Even => "|v| v % 2 == 0",
            Pred::Odd => "|v| v % 2 == 1",
            Pred::Zero => "|v| v == 0",
            Pred::NonZero => "|v| v != 0",
        }
    }
}

/// A gate's condition: a pre-instant bool cell, or its negation.
#[derive(Clone, Copy, Debug)]
struct Lit {
    cell: usize,
    negated: bool,
}

impl Lit {
    fn open(self, cells: u8) -> bool {
        (cells >> self.cell & 1 == 1) != self.negated
    }
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Map(MapFn, Src),
    Filter(Pred, Src),
    Gate(Lit, Src),
    Merge(Src, Src),
    OrElse(Src, Src),
    HoldSteps(Src),
}

impl Op {
    fn args(&self) -> Vec<Src> {
        match *self {
            Op::Map(_, a) | Op::Filter(_, a) | Op::Gate(_, a) | Op::HoldSteps(a) => vec![a],
            Op::Merge(a, b) | Op::OrElse(a, b) => vec![a, b],
        }
    }
}

#[derive(Clone, Debug)]
struct Program {
    nodes: Vec<Op>,
}

/// How a filter decides: by its predicate, or by a free outcome bit per
/// filter (the value-blind ablation).
#[derive(Clone, Copy)]
enum Filters {
    ByValue,
    Free(u32),
}

/// One instant's Kleene iteration from all ⊥ to the least fixpoint.
fn eval(p: &Program, inputs: &[V], cells: u8, filters: Filters, reverse: bool) -> Vec<V> {
    let n = p.nodes.len();
    let mut s = vec![V::Bot; n];
    let get = |s: &[V], a: Src| match a {
        Src::Input(i) => inputs[i],
        Src::Node(k) => s[k],
    };
    // Each node rises at most twice and a sweep that changes nothing
    // stops, so 2n + 1 sweeps reach the fixpoint.
    for _ in 0..=2 * n {
        let mut changed = false;
        for step in 0..n {
            let k = if reverse { n - 1 - step } else { step };
            let new = match p.nodes[k] {
                Op::Map(f, a) => match get(&s, a) {
                    V::Present(v) => V::Present(f.apply(v)),
                    other => other,
                },
                Op::HoldSteps(a) => get(&s, a),
                Op::Filter(pred, a) => {
                    let pass = match filters {
                        Filters::ByValue => None,
                        Filters::Free(bits) => {
                            let idx = p.nodes[..k]
                                .iter()
                                .filter(|o| matches!(o, Op::Filter(..)))
                                .count();
                            Some(bits >> idx & 1 == 1)
                        }
                    };
                    match (get(&s, a), pass) {
                        (V::Bot, _) => V::Bot,
                        (V::Absent, _) => V::Absent,
                        (x, Some(true)) => x,
                        (_, Some(false)) => V::Absent,
                        // The predicate on a ⊥ value is ⊥.
                        (V::PresentBot, None) => V::Bot,
                        (V::Present(v), None) => {
                            if pred.test(v) {
                                V::Present(v)
                            } else {
                                V::Absent
                            }
                        }
                    }
                }
                Op::Gate(lit, a) => {
                    if lit.open(cells) {
                        get(&s, a)
                    } else {
                        V::Absent
                    }
                }
                Op::Merge(a, b) => match (get(&s, a), get(&s, b)) {
                    (V::Absent, V::Absent) => V::Absent,
                    (V::Present(x), V::Present(y)) => V::Present((x + y) % VALUES),
                    (V::Present(x), V::Absent) | (V::Absent, V::Present(x)) => V::Present(x),
                    (x, y) if x.present() || y.present() => V::PresentBot,
                    _ => V::Bot,
                },
                Op::OrElse(a, b) => match (get(&s, a), get(&s, b)) {
                    (V::Absent, y) => y,
                    (x, _) if x.present() => x,
                    (_, y) if y.present() => V::PresentBot,
                    _ => V::Bot,
                },
            };
            if new != s[k] {
                s[k] = new;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    s
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Model {
    Any,
    AtLeastOne,
    ExactlyOne,
}

impl Model {
    fn name(self) -> &'static str {
        match self {
            Model::Any => "any",
            Model::AtLeastOne => ">=1",
            Model::ExactlyOne => "one",
        }
    }
    fn describe(self) -> &'static str {
        match self {
            Model::Any => "any presence combination, including the quiet instant",
            Model::AtLeastOne => "at least one input fires",
            Model::ExactlyOne => "exactly one input fires",
        }
    }
}

/// Every input combination the model allows: each input absent or present
/// with one of the values.
fn input_combos(n: usize, model: Model) -> Vec<Vec<V>> {
    let mut out = Vec::new();
    let per = VALUES as usize + 1;
    for code in 0..per.pow(n as u32) {
        let mut c = code;
        let mut row = Vec::with_capacity(n);
        for _ in 0..n {
            let d = c % per;
            c /= per;
            row.push(if d == 0 {
                V::Absent
            } else {
                V::Present(d as u8 - 1)
            });
        }
        let present = row.iter().filter(|v| v.present()).count();
        let ok = match model {
            Model::Any => true,
            Model::AtLeastOne => present >= 1,
            Model::ExactlyOne => present == 1,
        };
        if ok {
            out.push(row);
        }
    }
    out
}

/// Whether the node graph has a cycle, ignoring the edges `cut` names
/// (node, argument position).
fn has_cycle(p: &Program, cut: &dyn Fn(usize, usize) -> bool) -> bool {
    // 0 white, 1 grey, 2 black.
    fn visit(p: &Program, k: usize, mark: &mut [u8], cut: &dyn Fn(usize, usize) -> bool) -> bool {
        mark[k] = 1;
        for (pos, a) in p.nodes[k].args().into_iter().enumerate() {
            if let Src::Node(j) = a {
                if cut(k, pos) {
                    continue;
                }
                if mark[j] == 1 || (mark[j] == 0 && visit(p, j, mark, cut)) {
                    return true;
                }
            }
        }
        mark[k] = 2;
        false
    }
    let mut mark = vec![0u8; p.nodes.len()];
    (0..p.nodes.len()).any(|k| mark[k] == 0 && visit(p, k, &mut mark, cut))
}

fn gate_cut(p: &Program, cells: u8) -> impl Fn(usize, usize) -> bool + '_ {
    move |k, _| matches!(p.nodes[k], Op::Gate(lit, _) if !lit.open(cells))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Class {
    Gates,
    DeadBranch,
    PerInstant,
}

struct Verdict {
    class: Option<Class>,
    value_dependent: bool,
    order_mismatches: u64,
    /// Nodes present in no instant: a cycle nothing ever flows around.
    silent: usize,
}

fn judge(p: &Program, combos: &[Vec<V>]) -> Verdict {
    let mut order_mismatches = 0;
    let mut constructive = true;
    // Per cell state, per node: whether it's an `or_else` whose left side
    // was present in every instant of that state.
    let mut left_always = vec![vec![true; p.nodes.len()]; 1 << CELLS];
    let mut ever_present = vec![false; p.nodes.len()];
    for cells in 0..1u8 << CELLS {
        for inp in combos {
            let s = eval(p, inp, cells, Filters::ByValue, false);
            if s != eval(p, inp, cells, Filters::ByValue, true) {
                order_mismatches += 1;
            }
            if !s.iter().all(|v| v.settled()) {
                constructive = false;
            }
            for (e, v) in ever_present.iter_mut().zip(&s) {
                *e |= v.present();
            }
            for (k, op) in p.nodes.iter().enumerate() {
                if let Op::OrElse(a, _) = op {
                    let left = match *a {
                        Src::Input(i) => inp[i],
                        Src::Node(j) => s[j],
                    };
                    if !left.present() {
                        left_always[cells as usize][k] = false;
                    }
                }
            }
        }
    }
    let silent = ever_present.iter().filter(|e| !**e).count();
    if !constructive {
        return Verdict {
            class: None,
            value_dependent: false,
            order_mismatches,
            silent,
        };
    }
    let gates = (0..1u8 << CELLS).all(|cells| !has_cycle(p, &gate_cut(p, cells)));
    let class = if gates {
        Class::Gates
    } else if (0..1u8 << CELLS).all(|cells| {
        let g = gate_cut(p, cells);
        let dead = &left_always[cells as usize];
        !has_cycle(p, &|k, pos| g(k, pos) || (pos == 1 && dead[k]))
    }) {
        Class::DeadBranch
    } else {
        Class::PerInstant
    };
    let value_dependent = class != Class::Gates && {
        let filters = p
            .nodes
            .iter()
            .filter(|o| matches!(o, Op::Filter(..)))
            .count();
        !(0..1u32 << filters).all(|bits| {
            (0..1u8 << CELLS).all(|cells| {
                combos.iter().all(|inp| {
                    eval(p, inp, cells, Filters::Free(bits), false)
                        .iter()
                        .all(|v| v.settled())
                })
            })
        })
    };
    Verdict {
        class: Some(class),
        value_dependent,
        order_mismatches,
        silent,
    }
}

/// SplitMix64: small, seeded, good enough to draw programs.
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
}

/// Draws a program of `size` nodes. Each argument is an input with
/// probability 0.35, else any node, earlier, later or itself; a later or
/// the same node is a `stream_loop` forward.
fn draw(rng: &mut Rng, inputs: usize, size: usize) -> Program {
    let src = |rng: &mut Rng| {
        if rng.below(100) < 35 {
            Src::Input(rng.below(inputs))
        } else {
            Src::Node(rng.below(size))
        }
    };
    let nodes = (0..size)
        .map(|_| match rng.below(6) {
            0 => Op::Map(
                [MapFn::Inc, MapFn::Double, MapFn::Add3][rng.below(3)],
                src(rng),
            ),
            1 => Op::Filter(
                [Pred::Even, Pred::Odd, Pred::Zero, Pred::NonZero][rng.below(4)],
                src(rng),
            ),
            2 => Op::Gate(
                Lit {
                    cell: rng.below(CELLS),
                    negated: rng.below(2) == 1,
                },
                src(rng),
            ),
            3 => Op::Merge(src(rng), src(rng)),
            4 => Op::OrElse(src(rng), src(rng)),
            _ => Op::HoldSteps(src(rng)),
        })
        .collect();
    Program { nodes }
}

/// Draws an acyclic program: arguments only reach earlier nodes.
fn draw_acyclic(rng: &mut Rng, inputs: usize, size: usize) -> Program {
    let mut p = draw(rng, inputs, size);
    for (k, op) in p.nodes.iter_mut().enumerate() {
        let fix = |a: &mut Src, rng: &mut Rng| {
            if let Src::Node(j) = *a
                && j >= k
            {
                *a = if k == 0 {
                    Src::Input(rng.below(inputs))
                } else {
                    Src::Node(rng.below(k))
                };
            }
        };
        match op {
            Op::Map(_, a) | Op::Filter(_, a) | Op::Gate(_, a) | Op::HoldSteps(a) => fix(a, rng),
            Op::Merge(a, b) | Op::OrElse(a, b) => {
                fix(a, rng);
                fix(b, rng);
            }
        }
    }
    p
}

fn node_args(op: &Op) -> impl Iterator<Item = usize> {
    op.args().into_iter().filter_map(|a| match a {
        Src::Node(j) => Some(j),
        Src::Input(_) => None,
    })
}

/// Whether each node reaches itself: is on a cycle.
fn on_cycle(p: &Program) -> Vec<bool> {
    (0..p.nodes.len())
        .map(|k| {
            let mut seen = vec![false; p.nodes.len()];
            let mut stack: Vec<usize> = node_args(&p.nodes[k]).collect();
            while let Some(j) = stack.pop() {
                if j == k {
                    return true;
                }
                if !std::mem::replace(&mut seen[j], true) {
                    stack.extend(node_args(&p.nodes[j]));
                }
            }
            false
        })
        .collect()
}

/// Keeps only the nodes on a cycle and those upstream of one, renumbered.
/// Nodes downstream of every cycle settle whenever the cycles do, so
/// dropping them changes neither constructiveness nor the class; `main`
/// checks that for every example it prints.
fn prune(p: &Program) -> Program {
    let cyc = on_cycle(p);
    let mut keep = cyc.clone();
    let mut stack: Vec<usize> = (0..p.nodes.len()).filter(|&k| cyc[k]).collect();
    while let Some(k) = stack.pop() {
        for j in node_args(&p.nodes[k]) {
            if !std::mem::replace(&mut keep[j], true) {
                stack.push(j);
            }
        }
    }
    let mut index = vec![usize::MAX; p.nodes.len()];
    let mut next = 0;
    for (k, kept) in keep.iter().enumerate() {
        if *kept {
            index[k] = next;
            next += 1;
        }
    }
    let re = |a: Src| match a {
        Src::Node(j) => Src::Node(index[j]),
        other => other,
    };
    let nodes = p
        .nodes
        .iter()
        .zip(&keep)
        .filter(|(_, kept)| **kept)
        .map(|(op, _)| match *op {
            Op::Map(f, a) => Op::Map(f, re(a)),
            Op::Filter(pr, a) => Op::Filter(pr, re(a)),
            Op::Gate(l, a) => Op::Gate(l, re(a)),
            Op::HoldSteps(a) => Op::HoldSteps(re(a)),
            Op::Merge(a, b) => Op::Merge(re(a), re(b)),
            Op::OrElse(a, b) => Op::OrElse(re(a), re(b)),
        })
        .collect();
    Program { nodes }
}

/// Bough-like pseudocode: a node used at or before its definition is a
/// `stream_loop` forward, closed where it's defined.
fn show(p: &Program) -> String {
    let name = |a: Src| match a {
        Src::Input(i) => format!("i{i}"),
        Src::Node(k) => format!("s{k}"),
    };
    let mut out = String::new();
    let forward: Vec<bool> = (0..p.nodes.len())
        .map(|k| {
            p.nodes[..=k]
                .iter()
                .any(|o| o.args().contains(&Src::Node(k)))
        })
        .collect();
    for (k, f) in forward.iter().enumerate() {
        if *f {
            let _ = writeln!(out, "    let (s{k}, s{k}_loop) = b.stream_loop();");
        }
    }
    for (k, op) in p.nodes.iter().enumerate() {
        let expr = match *op {
            Op::Map(f, a) => format!("{}.map({})", name(a), f.text()),
            Op::Filter(pr, a) => format!("{}.filter({})", name(a), pr.text()),
            Op::Gate(l, a) => format!(
                "{}.gate({}c{})",
                name(a),
                if l.negated { "!" } else { "" },
                l.cell
            ),
            Op::Merge(a, c) => format!("{}.merge({}, |x, y| x + y)", name(a), name(c)),
            Op::OrElse(a, c) => format!("{}.or_else({})", name(a), name(c)),
            Op::HoldSteps(a) => format!("{}.hold(0).steps()", name(a)),
        };
        if forward[k] {
            let _ = writeln!(out, "    s{k}_loop.close({expr});");
        } else {
            let _ = writeln!(out, "    let s{k} = {expr};");
        }
    }
    out
}

#[derive(Default, Clone, Copy)]
struct Row {
    programs: u64,
    constructive: u64,
    gates: u64,
    dead: u64,
    per_instant: u64,
    value_dep: u64,
}

impl Row {
    fn add(&mut self, o: &Row) {
        self.programs += o.programs;
        self.constructive += o.constructive;
        self.gates += o.gates;
        self.dead += o.dead;
        self.per_instant += o.per_instant;
        self.value_dep += o.value_dep;
    }
    fn outside(&self) -> u64 {
        self.dead + self.per_instant
    }
}

fn pct(a: u64, b: u64) -> String {
    if b == 0 {
        "-".into()
    } else {
        format!("{:.2}%", 100.0 * a as f64 / b as f64)
    }
}

/// An example and its rank: whether some stream never fires (a cycle
/// nothing flows around sorts last), then pruned size.
type Example = Option<((bool, usize), Program)>;

struct Census {
    rows: Vec<[Row; 3]>,
    examples: [[Example; 3]; 3],
    value_example: [Example; 3],
    mismatches: u64,
}

const MODELS: [Model; 3] = [Model::Any, Model::AtLeastOne, Model::ExactlyOne];

fn census(inputs: usize, samples: usize, seed: u64) -> Census {
    let combos: Vec<Vec<Vec<V>>> = MODELS.iter().map(|&m| input_combos(inputs, m)).collect();
    let mut rng = Rng(seed);
    let mut c = Census {
        rows: Vec::new(),
        examples: Default::default(),
        value_example: Default::default(),
        mismatches: 0,
    };
    for size in 1..=MAX_SIZE {
        let mut rows = [Row::default(); 3];
        let mut drawn = 0;
        while drawn < samples {
            let p = draw(&mut rng, inputs, size);
            // Acyclicity refuses it: keep it.
            if !has_cycle(&p, &|_, _| false) {
                continue;
            }
            drawn += 1;
            let small = prune(&p);
            for m in 0..MODELS.len() {
                let v = judge(&p, &combos[m]);
                c.mismatches += v.order_mismatches;
                let r = &mut rows[m];
                r.programs += 1;
                let Some(class) = v.class else { continue };
                r.constructive += 1;
                let ci = match class {
                    Class::Gates => {
                        r.gates += 1;
                        0
                    }
                    Class::DeadBranch => {
                        r.dead += 1;
                        1
                    }
                    Class::PerInstant => {
                        r.per_instant += 1;
                        2
                    }
                };
                let rank = (v.silent > 0, small.nodes.len());
                let better = |e: &Example| e.as_ref().is_none_or(|(r0, _)| rank < *r0);
                if better(&c.examples[m][ci]) {
                    c.examples[m][ci] = Some((rank, small.clone()));
                }
                if v.value_dependent {
                    r.value_dep += 1;
                    if better(&c.value_example[m]) {
                        c.value_example[m] = Some((rank, small.clone()));
                    }
                }
            }
        }
        c.rows.push(rows);
    }
    c
}

fn row_line(label: &str, r: &Row) -> String {
    format!(
        "{:>4} {:>8} {:>12} {:>7} {:>8} {:>6} {:>6} {:>9} {:>9}",
        label,
        r.programs,
        r.constructive,
        pct(r.constructive, r.programs),
        r.gates,
        r.dead,
        r.per_instant,
        r.value_dep,
        pct(r.outside(), r.constructive)
    )
}

fn main() {
    println!(
        "ternary-loop-census: constructive cycles among those RFD 2's acyclicity rule refuses\n"
    );
    println!("Random programs over map, filter, gate (on c0, c1, !c0, !c1, read before the");
    println!("instant), merge (+), or_else and hold(0).steps(); values mod {VALUES}; {CELLS} free");
    println!("bool cells. Each kept program has a same-instant cycle. {SAMPLES} per size,");
    println!("2 inputs, seed 2026.");
    println!("(a)  gates: acyclic in every cell state once closed gates are cut.");
    println!("(b1) dead branch: acyclic only once an or_else's right edge is cut too, where");
    println!("     its left is present in every instant of that state.");
    println!("(b2) per instant: the cycle is live; what breaks it varies by instant.");
    println!("val: of (b), constructive only because of filter predicates on values.");
    println!("out: (b1 + b2) as a share of the constructive programs.\n");

    let c = census(2, SAMPLES, 2026);
    let mut totals = [Row::default(); 3];
    for (m, &model) in MODELS.iter().enumerate() {
        println!("input model `{}`: {}", model.name(), model.describe());
        println!(
            "{:>4} {:>8} {:>12} {:>7} {:>8} {:>6} {:>6} {:>9} {:>9}",
            "size", "programs", "constructive", "%", "(a)", "(b1)", "(b2)", "val", "out"
        );
        for (i, rows) in c.rows.iter().enumerate() {
            totals[m].add(&rows[m]);
            println!("{}", row_line(&(i + 1).to_string(), &rows[m]));
        }
        println!("{}\n", row_line("all", &totals[m]));
    }

    println!(
        "Sensitivity to the number of inputs ({} per size, all sizes):",
        SAMPLES / 5
    );
    println!(
        "{:>6} {:>5} {:>8} {:>12} {:>8} {:>6} {:>6}",
        "inputs", "model", "programs", "constructive", "(a)", "(b)", "val"
    );
    for inputs in [1, 3] {
        let other = census(inputs, SAMPLES / 5, 2026 + inputs as u64);
        for (m, &model) in MODELS.iter().enumerate() {
            let mut t = Row::default();
            for rows in &other.rows {
                t.add(&rows[m]);
            }
            println!(
                "{:>6} {:>5} {:>8} {:>12} {:>8} {:>6} {:>6}",
                inputs,
                model.name(),
                t.programs,
                t.constructive,
                t.gates,
                t.outside(),
                t.value_dep
            );
        }
    }
    println!();

    // Controls: acyclic programs are all constructive, and the fixpoint
    // doesn't depend on sweep order.
    let combos = input_combos(2, Model::Any);
    let mut rng = Rng(7);
    let (mut acyclic, mut ok) = (0, 0);
    for size in 1..=MAX_SIZE {
        for _ in 0..2_000 {
            let p = draw_acyclic(&mut rng, 2, size);
            acyclic += 1;
            if judge(&p, &combos).class.is_some() {
                ok += 1;
            }
        }
    }
    println!("control: {ok} of {acyclic} acyclic programs constructive under `any`");
    println!(
        "control: {} instants where forward and reverse sweeps disagreed\n",
        c.mismatches
    );

    println!("Minimal examples, 2 inputs: every stream fires in some instant if any such");
    println!("was found, then fewest nodes once those downstream of every cycle are dropped.");
    println!("Each is re-judged after dropping them, to check it kept its class.\n");
    let all_combos: Vec<Vec<Vec<V>>> = MODELS.iter().map(|&m| input_combos(2, m)).collect();
    let print_example = |label: &str, m: usize, e: &Example, want: &dyn Fn(&Verdict) -> bool| {
        let name = MODELS[m].name();
        let Some((_, p)) = e else {
            println!("{label}, model `{name}`: none found\n");
            return;
        };
        let v = judge(p, &all_combos[m]);
        println!(
            "{label}, model `{name}` (class kept: {}; every stream fires: {}):",
            if want(&v) { "yes" } else { "NO" },
            if v.silent == 0 { "yes" } else { "no" }
        );
        println!("{}", show(p));
    };
    let labels = ["(a) gates", "(b1) dead branch", "(b2) per instant"];
    let classes = [Class::Gates, Class::DeadBranch, Class::PerInstant];
    for (ci, label) in labels.iter().enumerate() {
        for m in 0..MODELS.len() {
            // (a) is the same class under every model: print it once.
            if ci == 0 && m > 0 {
                continue;
            }
            print_example(label, m, &c.examples[m][ci], &|v| {
                v.class == Some(classes[ci])
            });
        }
    }
    for m in 0..MODELS.len() {
        print_example("(b) value-dependent", m, &c.value_example[m], &|v| {
            v.value_dependent
        });
    }

    let [any, one_plus, one] = totals;
    println!(
        "verdict: of {} refused programs, {} were constructive under `any`, {} of them outside (a); \
         {} under `>=1`, {} outside (a), {} value-dependent; {} under `one`, {} outside (a), {} value-dependent.",
        any.programs,
        any.constructive,
        any.outside(),
        one_plus.constructive,
        one_plus.outside(),
        one_plus.value_dep,
        one.constructive,
        one.outside(),
        one.value_dep
    );
}
