//! Does Bough's creation cut make every primitive forgetful, where the
//! semantics text's replay is not, and does it matter whether the cut is on
//! every constructed primitive or only on state-holders and time-movers?
//!
//! RFD 1 follows Sodium's denotational semantics (the book's App. E) except
//! where the text breaks its own rules; the review now justifies F89's cut
//! by FRPNow's forgetfulness (van der Ploeg and Claessen, ICFP 2015,
//! Lemmas 1 and 2): an operation that takes its start from the past is
//! inherently leaky, one tied to now is forgetful. This probe models App.
//! E's loop-free primitives over finite `[Int]`-timed lists (`model`), in
//! three variants (`prog::Variant`): the text as written, a cut on every
//! primitive built at t0 (option a of `13-sodium.md`), and a cut on
//! `hold`, `value`, `switch_cell` and `split`/`defer` only (option b).
//!
//! Forgetfulness, stated for this model in FRPNow's Kripke form: a node
//! built at t0 gives the same output from t0 on for any two argument
//! histories that agree from t0 on. "Agree from t0 on" is the same events
//! at t >= t0 for a stream, and the same value at t0 and steps at t >= t0
//! for a cell. The literal form, over the program's input histories, is
//! false even for a hold built at [0] and sampled later, since older state
//! legitimately remembers; the probe measures it too, to show that.
//!
//! Checks, on random loop-free programs with `construct` building nodes at
//! child instants:
//!
//! 1. Local: rebuild each node over its arguments chopped at t0, and over
//!    random histories that agree with them from t0 on but carry junk
//!    before it. "Leaks" is the chop differing; "unforgetful" is any of
//!    them differing.
//! 2. Whole program: rerun on an input history that differs only before
//!    some instant, and compare each node built at or after it whose
//!    arguments agree from its t0 on.
//! 3. Option (a) against option (b), node by node from each node's t0.
//! 4. The text against option (b): the nodes where the divergence starts
//!    (arguments equal from t0 on, output not), by shape.
//!
//! Plus the drafts' reproductions of F6, F7 and F89.

// A crate root's modules resolve beside it, not under its stem.
#[path = "rfd-0001-forgetful-cut/model.rs"]
mod model;
#[path = "rfd-0001-forgetful-cut/prog.rs"]
mod prog;

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use model::{Den, F2, Fan, Kind, Node, S, T, V, at, eq_from, from, in_order, node, show, show_t};
use prog::{Ctx, Id, Op, Prog, Reg, Rng, Ty, Variant};

const PROGRAMS: u64 = 3000;
const HISTORIES: usize = 2;
const JUNK_TRIES: usize = 3;

// ---------------------------------------------------------------------------
// Perturbation: an argument history that agrees with the real one from t0
// on, with junk before it.

/// Times before t0 a history could have used: external instants, their
/// first children, and t0's own ancestors and their earlier children.
fn pool(t0: &T) -> Vec<T> {
    let mut p = vec![vec![0]];
    for k in 1..=t0[0] {
        p.push(vec![k]);
        for j in 0..3 {
            p.push(vec![k, j]);
        }
    }
    for l in 1..=t0.len() {
        let pre = t0[..l].to_vec();
        for j in 0..3 {
            let mut c = pre.clone();
            c.push(j);
            p.push(c);
        }
        p.push(pre);
    }
    p.retain(|t| t < t0);
    p.sort();
    p.dedup();
    p
}

/// A random value of a history's element type. Junk cells and streams
/// change at times around t0, so that a leak of one is visible.
fn junk(rng: &mut Rng, ty: Ty, t0: &T) -> V {
    let mut near = pool(t0);
    near.push(t0.clone());
    near.push(vec![t0[0] + 1]);
    let t = near[rng.below(near.len())].clone();
    match ty {
        Ty::SI | Ty::CI => V::I(rng.int(-9, 99)),
        Ty::SC | Ty::CC => V::N(Node::new(Kind::Concrete(
            V::I(rng.int(-9, 99)),
            vec![(t, V::I(rng.int(-9, 99)))],
        ))),
        Ty::CS => V::N(Node::new(Kind::Mk(vec![(t, V::I(rng.int(-9, 99)))]))),
    }
}

fn perturb(rng: &mut Rng, v: &V, ty: Ty, t0: &T, chop: bool) -> V {
    let n = node(v);
    let mut times = Vec::new();
    if !chop {
        let p = pool(t0);
        if !p.is_empty() {
            for _ in 0..rng.int(1, 3) {
                times.push(p[rng.below(p.len())].clone());
            }
        }
        times.sort();
        times.dedup();
    }
    let mut junk_s: S = times.into_iter().map(|t| (t, junk(rng, ty, t0))).collect();
    V::N(Node::new(match n.den() {
        Den::C(i, s) => {
            // Keep the value at t0: the last junk step, or the initial value.
            let cur = at(i, s, t0);
            let init = match junk_s.last_mut() {
                Some(last) => {
                    last.1 = cur;
                    junk(rng, ty, t0)
                }
                None => cur,
            };
            junk_s.extend(from(s, t0));
            Kind::Concrete(init, junk_s)
        }
        Den::S(s) => {
            junk_s.extend(from(s, t0));
            Kind::Mk(junk_s)
        }
    }))
}

/// Rebuilds a node over perturbed arguments. `None` if forgetful on every
/// try, else which try broke it: 0 is the chop.
fn check_local(rng: &mut Rng, ctx: &Rc<Ctx>, r: &Reg) -> Option<usize> {
    if r.args.is_empty() {
        return None;
    }
    (0..=JUNK_TRIES).find(|&k| {
        let args: Vec<V> = r
            .args
            .iter()
            .zip(&r.arg_tys)
            .map(|(a, ty)| perturb(rng, a, *ty, &r.t0, k == 0))
            .collect();
        let out = prog::make(ctx, &r.op, &args, &r.t0, &r.scope, &r.id.0, r.id.1);
        !eq_from(&r.out, &out, &r.t0)
    })
}

// ---------------------------------------------------------------------------
// Shapes.

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Shape {
    /// A `switch_cell` whose outer stepped before the switch was built.
    F6,
    /// A `split`/`defer` whose input fired, before it was built, at an
    /// instant whose children fall at or after it.
    F89,
    /// A `split` whose output is out of time order.
    F7,
    /// An argument out of time order: downstream of F6 or F7 in the text.
    Downstream,
    /// A `construct` whose body built an F6-, F89- or F7-shaped node, so
    /// its cells differ from inside.
    Body,
    Other,
}

fn shape(r: &Reg) -> Shape {
    let sh = own_shape(r);
    if sh == Shape::Other && !args_in_order(r) {
        return Shape::Downstream;
    }
    sh
}

fn args_in_order(r: &Reg) -> bool {
    r.args.iter().all(|a| in_order(node(a)))
}

/// The worse of a node's shapes in two runs, looking into a construct's
/// bodies in both runs.
fn shape2(r: &Reg, c: &Ctx, r2: &Reg, c2: &Ctx) -> Shape {
    let sh = shape(r).min(shape(r2));
    if sh == Shape::Other && (in_body_shaped(r, c) || in_body_shaped(r2, c2)) {
        return Shape::Body;
    }
    sh
}

/// Whether any node a construct's bodies built, nested ones included, has
/// one of the text's shapes.
fn in_body_shaped(r: &Reg, ctx: &Ctx) -> bool {
    if !matches!(r.op, Op::Construct(..)) {
        return false;
    }
    let depth = r.id.0.len();
    ctx.reg.borrow().iter().any(|b| {
        b.id.0.len() > depth
            && b.id.0[..depth] == r.id.0[..]
            && b.id.0[depth].0 == r.id.1
            && matches!(own_shape(b), Shape::F6 | Shape::F89 | Shape::F7)
    })
}

fn own_shape(r: &Reg) -> Shape {
    match &r.op {
        Op::SwitchCell(_) => {
            let (_, s) = node(&r.args[0]).steps();
            if s.iter().any(|(t, _)| t < &r.t0) {
                return Shape::F6;
            }
        }
        Op::Split(_, fan) => {
            let last = fan.len() as i32 - 1;
            let replays = node(&r.args[0]).occs().iter().any(|(t, _)| {
                let mut c = t.clone();
                c.push(last);
                t < &r.t0 && c >= r.t0
            });
            if replays {
                return Shape::F89;
            }
            if !in_order(node(&r.out)) {
                return Shape::F7;
            }
        }
        _ => {}
    }
    Shape::Other
}

/// Where a node was built: at [0], at an external instant, or at a child.
fn when(t0: &T) -> &'static str {
    match t0.len() {
        1 if t0[0] == 0 => "build",
        1 => "external",
        _ => "child",
    }
}

// ---------------------------------------------------------------------------
// Tallies.

#[derive(Default)]
struct Tally {
    nodes: usize,
    /// Nodes built after [0] with arguments: the ones that can fail.
    late: usize,
    leaks: BTreeMap<(Shape, String), usize>,
    not_forgetful: BTreeMap<String, usize>,
    /// Nodes out of time order whose arguments are in order: where it starts.
    out_of_order: BTreeMap<String, usize>,
    out_of_order_all: usize,
    runs_leaking: usize,
    /// Late nodes checked, by op and where built.
    coverage: BTreeMap<String, usize>,
    shaped: BTreeMap<Shape, (usize, usize)>,
    whole_premise: usize,
    whole_literal_diff: usize,
    whole_violations: BTreeMap<(Shape, String), usize>,
}

/// The smallest failing program seen for a label.
struct Example {
    size: usize,
    seed: u64,
    hist: Vec<S>,
    node: Id,
    t0: T,
    note: String,
}

fn keep(ex: &mut BTreeMap<String, Example>, label: String, e: Example) {
    if ex.get(&label).is_none_or(|old| e.size < old.size) {
        ex.insert(label, e);
    }
}

fn label(r: &Reg) -> String {
    format!("{} at {}", r.op.name(), when(&r.t0))
}

fn show_id(id: &Id) -> String {
    let mut s = String::new();
    for (c, t) in &id.0 {
        s += &format!("body of r{c} at {} / ", show_t(t));
    }
    s + &format!("its op {}", id.1)
}

fn index(ctx: &Rc<Ctx>) -> HashMap<Id, usize> {
    ctx.reg
        .borrow()
        .iter()
        .enumerate()
        .map(|(i, r)| (r.id.clone(), i))
        .collect()
}

/// Whether two builds of one node read the same from its t0 on: its
/// arguments and, for a `construct`, what its body captures.
fn same_premise(r: &Reg, r2: &Reg) -> bool {
    let args = r
        .args
        .iter()
        .zip(&r2.args)
        .all(|(a, b)| eq_from(a, b, &r.t0));
    let caps = match &r.op {
        Op::Construct(_, body) => {
            let mut c = Vec::new();
            prog::captured(body, r.scope.vals.len(), &mut c);
            c.iter()
                .all(|&i| eq_from(&r.scope.vals[i], &r2.scope.vals[i], &r.t0))
        }
        _ => true,
    };
    args && caps
}

/// A history that agrees with `h` at external times from `[k]` on.
fn rehistory(rng: &mut Rng, h: &[S], k: i32) -> Vec<S> {
    let mut fresh = prog::gen_inputs(rng);
    for (f, old) in fresh.iter_mut().zip(h) {
        f.retain(|(t, _)| t[0] < k);
        f.extend(old.iter().filter(|(t, _)| t[0] >= k).cloned());
    }
    fresh
}

fn inputs_agree(a: &[S], b: &[S], t0: &T) -> bool {
    a.iter().zip(b).all(|(x, y)| model::eq_s_from(x, y, t0))
}

// ---------------------------------------------------------------------------

fn main() {
    println!("forgetful-cut: the text's replay against Bough's creation cut, over T = [Int]\n");
    reproductions();

    let mut tally: BTreeMap<Variant, Tally> = BTreeMap::new();
    let mut examples: BTreeMap<String, Example> = BTreeMap::new();
    let (mut ab_nodes, mut ab_diff, mut only_a, mut only_b) = (0, 0, 0, 0);
    let mut origins: BTreeMap<(Shape, String), usize> = BTreeMap::new();
    let mut runs_total = 0;

    for seed in 0..PROGRAMS {
        let mut rng = Rng::new(seed);
        let p = prog::gen_prog(&mut rng);
        let size = prog::size(&p.ops);
        for _ in 0..HISTORIES {
            let h = prog::gen_inputs(&mut rng);
            let mut runs: BTreeMap<Variant, Rc<Ctx>> = BTreeMap::new();
            for v in Variant::ALL {
                let ctx = prog::run(&p, &h, v);
                let t = tally.entry(v).or_default();
                let mut leaked = false;
                for r in ctx.reg.borrow().iter() {
                    t.nodes += 1;
                    if !in_order(node(&r.out)) {
                        t.out_of_order_all += 1;
                        if args_in_order(r) {
                            *t.out_of_order.entry(label(r)).or_default() += 1;
                        }
                    }
                    if r.args.is_empty() || r.t0 == vec![0] {
                        continue;
                    }
                    t.late += 1;
                    let sh = shape(r);
                    let failed = check_local(&mut rng, &ctx, r);
                    if matches!(sh, Shape::F6 | Shape::F89) {
                        let e = t.shaped.entry(sh).or_default();
                        e.0 += 1;
                        e.1 += usize::from(failed == Some(0));
                    }
                    *t.coverage.entry(label(r)).or_default() += 1;
                    let Some(k) = failed else { continue };
                    *t.not_forgetful.entry(label(r)).or_default() += 1;
                    // A junk past fails the text's switch_cell and split at
                    // most child instants; only the runs' own leaks are
                    // worth an example.
                    if k != 0 && v == Variant::Text {
                        continue;
                    }
                    let kind = if k == 0 { "leaks" } else { "unforgetful" };
                    if k == 0 {
                        leaked = true;
                        *t.leaks.entry((sh, label(r))).or_default() += 1;
                    }
                    keep(
                        &mut examples,
                        format!("{} {kind}: {sh:?}, {}", v.name(), label(r)),
                        Example {
                            size,
                            seed,
                            hist: h.clone(),
                            node: r.id.clone(),
                            t0: r.t0.clone(),
                            note: show(node(&r.out)),
                        },
                    );
                }
                t.runs_leaking += usize::from(leaked);
                whole_program(&mut rng, &p, &h, &ctx, t);
                runs.insert(v, ctx);
            }
            runs_total += 1;

            // Option (a) against option (b).
            let (a, b) = (&runs[&Variant::CutAll], &runs[&Variant::CutStateful]);
            let (ia, ib) = (index(a), index(b));
            only_a += ia.keys().filter(|k| !ib.contains_key(*k)).count();
            only_b += ib.keys().filter(|k| !ia.contains_key(*k)).count();
            for r in a.reg.borrow().iter() {
                let Some(&j) = ib.get(&r.id) else { continue };
                ab_nodes += 1;
                let rb = &b.reg.borrow()[j];
                if !eq_from(&r.out, &rb.out, &r.t0) {
                    ab_diff += 1;
                    keep(
                        &mut examples,
                        format!("cut-all differs from cut-stateful: {}", label(r)),
                        Example {
                            size,
                            seed,
                            hist: h.clone(),
                            node: r.id.clone(),
                            t0: r.t0.clone(),
                            note: format!("{} vs {}", show(node(&r.out)), show(node(&rb.out))),
                        },
                    );
                }
            }

            // Where the text starts to differ from option (b).
            let x = &runs[&Variant::Text];
            for r in x.reg.borrow().iter() {
                let Some(&j) = ib.get(&r.id) else { continue };
                let rb = &b.reg.borrow()[j];
                if same_premise(r, rb) && !eq_from(&r.out, &rb.out, &r.t0) {
                    let sh = shape2(r, x, rb, b);
                    *origins.entry((sh, label(r))).or_default() += 1;
                    if sh == Shape::Other {
                        keep(
                            &mut examples,
                            format!("text diverges, other shape: {}", label(r)),
                            Example {
                                size,
                                seed,
                                hist: h.clone(),
                                node: r.id.clone(),
                                t0: r.t0.clone(),
                                note: format!("{} vs {}", show(node(&r.out)), show(node(&rb.out))),
                            },
                        );
                    }
                }
            }
            for ctx in runs.values() {
                prog::finish(ctx);
            }
        }
    }

    report(
        &tally,
        runs_total,
        (ab_nodes, ab_diff, only_a, only_b),
        &origins,
    );
    println!("\nSmallest program per finding:\n");
    for (l, e) in &examples {
        let mut rng = Rng::new(e.seed);
        let p = prog::gen_prog(&mut rng);
        let mut text = String::new();
        prog::show_prog(&p.ops, 0, 4, &mut text);
        println!("{l}");
        println!(
            "  seed {}, {} ops; node: {}, built at {}",
            e.seed,
            e.size,
            show_id(&e.node),
            show_t(&e.t0)
        );
        for (i, s) in e.hist.iter().enumerate() {
            println!("  input {i}: [{}]", model::show_s(s));
        }
        print!("{text}");
        println!("  output: {}\n", e.note);
    }
}

/// Check 2: rerun on a history that differs only before `[k]`.
fn whole_program(rng: &mut Rng, p: &Prog, h: &[S], ctx: &Rc<Ctx>, t: &mut Tally) {
    let k = rng.int(2, prog::TXS as i64) as i32;
    let h2 = rehistory(rng, h, k);
    let ctx2 = prog::run(p, &h2, ctx.variant);
    let i2 = index(&ctx2);
    for r in ctx.reg.borrow().iter() {
        if r.t0 == vec![0] || r.args.is_empty() || !inputs_agree(h, &h2, &r.t0) {
            continue;
        }
        let Some(&j) = i2.get(&r.id) else { continue };
        let r2 = &ctx2.reg.borrow()[j];
        let outs_same = eq_from(&r.out, &r2.out, &r.t0);
        t.whole_literal_diff += usize::from(!outs_same);
        if same_premise(r, r2) {
            t.whole_premise += 1;
            if !outs_same {
                let sh = shape2(r, ctx, r2, &ctx2);
                *t.whole_violations.entry((sh, label(r))).or_default() += 1;
            }
        }
    }
    prog::finish(&ctx2);
}

fn report(
    tally: &BTreeMap<Variant, Tally>,
    runs: usize,
    (ab_nodes, ab_diff, only_a, only_b): (usize, usize, usize, usize),
    origins: &BTreeMap<(Shape, String), usize>,
) {
    println!(
        "Random programs: {PROGRAMS} programs x {HISTORIES} input histories = {runs} runs; \
         {} inputs firing at [1]..[{}]",
        prog::INPUTS,
        prog::TXS
    );
    println!(
        "Local check: each node built after [0] rebuilt over its arguments chopped at t0, \
         and over {JUNK_TRIES} junk pasts\n"
    );
    println!(
        "{:<13} {:>7} {:>7} {:>10} {:>10} {:>12} {:>11} {:>8}",
        "variant", "nodes", "late", "leak runs", "leak nodes", "unforgetful", "whole", "literal"
    );
    for (v, t) in tally {
        let leak_nodes: usize = t.leaks.values().sum();
        let unf: usize = t.not_forgetful.values().sum();
        let whole: usize = t.whole_violations.values().sum();
        println!(
            "{:<13} {:>7} {:>7} {:>10} {:>10} {:>12} {:>11} {:>8}",
            v.name(),
            t.nodes,
            t.late,
            t.runs_leaking,
            leak_nodes,
            unf,
            format!("{whole}/{}", t.whole_premise),
            t.whole_literal_diff
        );
    }
    println!(
        "\n  nodes: every node the runs built; late: those built after [0] with arguments\n  \
         leak runs / leak nodes: output from t0 changes when the arguments' past is chopped\n  \
         unforgetful: output from t0 changes under the chop or a junk past\n  \
         whole: rerun on inputs that agree from t0 on; nodes whose output changed, of those whose \
         arguments also agree\n  \
         literal: the same rerun, nodes whose output changed with only the inputs agreeing"
    );
    for (v, t) in tally {
        println!("\n{}:", v.name());
        for ((sh, l), n) in &t.leaks {
            println!("  leaks          {:<6} {l:<26} {n}", format!("{sh:?}"));
        }
        for (l, n) in &t.not_forgetful {
            println!("  unforgetful           {l:<26} {n}");
        }
        for ((sh, l), n) in &t.whole_violations {
            println!("  whole          {:<6} {l:<26} {n}", format!("{sh:?}"));
        }
        for (l, n) in &t.out_of_order {
            println!("  out of order (origin) {l:<26} {n}");
        }
        if t.out_of_order_all > 0 {
            println!("  out of order, all nodes: {}", t.out_of_order_all);
        }
        for (sh, (n, bad)) in &t.shaped {
            println!("  {sh:?}-shaped nodes: {n}, of which leak: {bad}");
        }
    }
    println!("\nLate nodes checked under cut-stateful, by op and where built:");
    let cov: Vec<String> = tally[&Variant::CutStateful]
        .coverage
        .iter()
        .map(|(l, n)| format!("{l} {n}"))
        .collect();
    for line in cov.chunks(4) {
        println!("  {}", line.join(", "));
    }
    println!(
        "\nText against cut-stateful, nodes where they part (arguments equal from t0, output not):"
    );
    for ((sh, l), n) in origins {
        println!("  {:<6} {l:<26} {n}", format!("{sh:?}"));
    }
    println!(
        "\nCut-all against cut-stateful: {ab_nodes} nodes in both, {ab_diff} differ from their t0 on; \
         {only_a} only in cut-all, {only_b} only in cut-stateful (bodies run for events before \
         their construct existed)"
    );
    let text = &tally[&Variant::Text];
    let text_count = |want: &[Shape]| -> usize {
        text.leaks
            .iter()
            .chain(&text.whole_violations)
            .filter(|((s, _), _)| want.contains(s))
            .map(|(_, n)| n)
            .sum()
    };
    let text_shaped = text_count(&[Shape::F6, Shape::F89]);
    let text_down = text_count(&[Shape::Downstream, Shape::Body]);
    let text_other = text_count(&[Shape::Other, Shape::F7]);
    let cut_fail: usize = [Variant::CutAll, Variant::CutStateful]
        .iter()
        .map(|v| {
            let t = &tally[v];
            t.not_forgetful.values().sum::<usize>() + t.whole_violations.values().sum::<usize>()
        })
        .sum();
    println!(
        "\nVerdict: text leaks in {} of {runs} runs: {text_shaped} nodes of F6/F89 shape, {text_down} \
         downstream of one (an out-of-order argument, or a construct whose body has one), \
         {text_other} otherwise; the two cuts fail forgetfulness at {cut_fail} nodes; cut-all and \
         cut-stateful differ at {ab_diff} of {ab_nodes} shared nodes.",
        text.runs_leaking
    );
}

// ---------------------------------------------------------------------------
// The drafts' reproductions, as programs.

type Case = (&'static str, &'static str, Prog, Vec<S>, Id);

fn reproductions() {
    let inp = |ts: &[(i32, i64)]| -> S { ts.iter().map(|&(k, v)| (vec![k], V::I(v))).collect() };
    let body = |ops: Vec<Op>, ret: usize| Rc::new(prog::BodyR { ops, ret });
    let cases: Vec<Case> = vec![
        (
            "F6",
            "a switch_cell built at [3] inside construct, over an outer that picked c2 at [1];\n    \
             c1 = constant 1; c2 = hold 10, steps to 20 at [3] (the draft's 'a', 'x', 'y')",
            Prog {
                ops: vec![
                    Op::Input(0),
                    Op::Input(1),
                    Op::Constant(1),
                    Op::Hold(1, 10),
                    Op::Hold(0, 0),
                    Op::Pick(4, vec![2, 3]),
                    Op::Construct(1, body(vec![Op::SwitchCell(5)], 7)),
                    Op::HoldC(6, 2),
                    Op::SwitchCell(7),
                ],
            },
            vec![inp(&[(1, 1)]), inp(&[(3, 20)])],
            (vec![(6, vec![3])], 0),
        ),
        (
            "F7",
            "a split into 2 of an input (1 at [1]) merged with its own defer",
            Prog {
                ops: vec![
                    Op::Input(0),
                    Op::Input(1),
                    Op::Split(0, Fan::One),
                    Op::Merge(0, 2, F2::Add),
                    Op::Split(3, Fan::Spread(2)),
                ],
            },
            vec![inp(&[(1, 1)]), vec![]],
            (vec![], 4),
        ),
        (
            "F89",
            "construct over defer x runs at [1,0]; the body splits x into 3 and holds -1 over them",
            Prog {
                ops: vec![
                    Op::Input(0),
                    Op::Input(1),
                    Op::Split(0, Fan::One),
                    Op::Constant(0),
                    Op::Construct(
                        2,
                        body(vec![Op::Split(0, Fan::Spread(3)), Op::Hold(5, -1)], 6),
                    ),
                    Op::HoldC(4, 3),
                    Op::SwitchCell(5),
                ],
            },
            vec![inp(&[(1, 1)]), vec![]],
            (vec![(4, vec![1, 0])], 0),
        ),
        (
            "F89 defer",
            "the same with a defer of x in the body",
            Prog {
                ops: vec![
                    Op::Input(0),
                    Op::Input(1),
                    Op::Split(0, Fan::One),
                    Op::Constant(0),
                    Op::Construct(2, body(vec![Op::Split(0, Fan::One), Op::Hold(5, -1)], 6)),
                    Op::HoldC(4, 3),
                    Op::SwitchCell(5),
                ],
            },
            vec![inp(&[(1, 1)]), vec![]],
            (vec![(4, vec![1, 0])], 0),
        ),
    ];
    println!(
        "Reproductions (the node is the one the finding is about; 'program' is the last node):\n"
    );
    let mut rng = Rng::new(0xF0F0);
    for (name, what, p, h, id) in cases {
        println!("{name}: {what}");
        for v in Variant::ALL {
            let ctx = prog::run(&p, &h, v);
            {
                let reg = ctx.reg.borrow();
                let r = reg
                    .iter()
                    .find(|r| r.id == id)
                    .expect("node in the reproduction");
                let last = &reg.iter().rfind(|r| r.id.0.is_empty()).unwrap().out;
                let leak = check_local(&mut rng, &ctx, r);
                println!(
                    "  {:<13} {} at {}: {}  in order: {}  forgetful: {}\n  {:<13} program: {}",
                    v.name(),
                    r.op.name(),
                    show_t(&r.t0),
                    show(node(&r.out)),
                    if in_order(node(&r.out)) { "yes" } else { "NO" },
                    match leak {
                        None => "yes",
                        Some(0) => "NO (the chop)",
                        Some(_) => "NO (a junk past)",
                    },
                    "",
                    show(node(last)),
                );
            }
            prog::finish(&ctx);
        }
        println!();
    }
}
