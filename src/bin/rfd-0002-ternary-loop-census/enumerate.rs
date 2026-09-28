//! Enumerated mode (`--enumerate`): does a constructive loop outside the
//! exclusive gates exist with a cell that changes and two or more inputs?
//!
//! The reachable-state mode drew one cell binding per random program. It
//! found (c) programs with a changing cell only with a single input, and a
//! draw can miss a rare binding. So here every program of 2 to 4 nodes is
//! enumerated, and for each every binding of each gate cell: `src` any input
//! or stream of the program, `pred` one of the four filter predicates,
//! `init` false or true. Each binding's reachable cell states are explored
//! and the program classified as in the reachable-state mode, under each
//! input model, with 2 and with 3 inputs.
//!
//! Choices the question left open:
//!
//! - **Programs are counted up to symmetry:** renaming nodes, permuting
//!   inputs, swapping the two cells, and negating a cell (a gate on `!c`
//!   with `c = src.map(pred).hold(init)` is a gate on `c'` with
//!   `c' = src.map(!pred).hold(!init)`, and the binding enumeration covers
//!   both). `merge` with `+` is commutative, so its arguments are sorted.
//!   Each orbit is judged once, by its least encoding. None of these changes
//!   constructiveness or the class, and a control checks the enumerator
//!   against brute force on the small sizes.
//! - **Only what depends on the cells is done per binding.** Whether every
//!   instant settles in a cell state is a property of the program and the
//!   state, so it is computed once per state; a binding then only decides
//!   which states are reached. The class depends only on the reached set
//!   (`classify_reached`), so it is computed once per set. Every (c) binding
//!   is re-judged by `judge_reach`, as are a sample of all bindings, and the
//!   two must agree.
//! - A gate cell no gate reads is left constant: it can't change anything.

use super::*;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

const MIN_SIZE: usize = 2;
const MAX_ENUM: usize = 4;
/// One binding in this many is re-judged by `judge_reach` as a control.
const SAMPLE_EVERY: u64 = 211;

const MAPS: [MapFn; 3] = [MapFn::Inc, MapFn::Double, MapFn::Add3];
const PREDS: [Pred; 4] = [Pred::Even, Pred::Odd, Pred::Zero, Pred::NonZero];

/// The binding of a cell no gate reads. Its value never matters.
const UNUSED: CellDef = CellDef {
    src: Src::Input(0),
    pred: Pred::Even,
    init: false,
};

fn kind(op: &Op) -> u32 {
    match op {
        Op::Map(..) => 0,
        Op::Filter(..) => 1,
        Op::Gate(..) => 2,
        Op::Merge(..) => 3,
        Op::OrElse(..) => 4,
        Op::HoldSteps(..) => 5,
    }
}

fn src_code(a: Src) -> u32 {
    match a {
        Src::Input(i) => i as u32,
        Src::Node(k) => 8 + k as u32,
    }
}

fn pred_index(p: Pred) -> u32 {
    PREDS.iter().position(|q| q.text() == p.text()).unwrap() as u32
}

fn complement(p: Pred) -> Pred {
    match p {
        Pred::Even => Pred::Odd,
        Pred::Odd => Pred::Even,
        Pred::Zero => Pred::NonZero,
        Pred::NonZero => Pred::Zero,
    }
}

/// A node's encoding, ordered by kind first, so that a program whose kinds
/// don't ascend is never the least of its orbit. `lit` is the gate's
/// literal after renaming cells; merge arguments are sorted.
fn code(op: &Op, lit: Option<Lit>, re: &dyn Fn(Src) -> Src) -> u32 {
    let (param, a, b) = match *op {
        Op::Map(f, a) => (
            MAPS.iter().position(|g| g.text() == f.text()).unwrap() as u32,
            re(a),
            None,
        ),
        Op::Filter(p, a) => (pred_index(p), re(a), None),
        Op::Gate(_, a) => {
            let l = lit.unwrap();
            (l.cell as u32 * 2 + l.negated as u32, re(a), None)
        }
        Op::Merge(a, b) => {
            let (x, y) = (re(a), re(b));
            if src_code(x) <= src_code(y) {
                (0, x, Some(y))
            } else {
                (0, y, Some(x))
            }
        }
        Op::OrElse(a, b) => (0, re(a), Some(re(b))),
        Op::HoldSteps(a) => (0, re(a), None),
    };
    kind(op) << 16 | param << 12 | (src_code(a) + 1) << 6 | b.map_or(0, |b| src_code(b) + 1)
}

type Key = [u32; MAX_ENUM];

/// The encoding of `p` with its nodes placed by `pi` (old node k becomes
/// node `pi[k]`) and inputs by `sigma`, cells renamed in order of first use
/// and each cell's first gate made un-negated.
fn perm_key(p: &Program, pi: &[usize], sigma: &[usize]) -> Key {
    let n = p.nodes.len();
    let mut inv = [0; MAX_ENUM];
    for (k, &j) in pi.iter().enumerate() {
        inv[j] = k;
    }
    let re = |a: Src| match a {
        Src::Input(i) => Src::Input(sigma[i]),
        Src::Node(k) => Src::Node(pi[k]),
    };
    let mut cells: [Option<(usize, bool)>; CELLS] = [None; CELLS];
    let mut used = 0;
    let mut key = [0; MAX_ENUM];
    for (j, slot) in key.iter_mut().enumerate().take(n) {
        let op = &p.nodes[inv[j]];
        let lit = if let Op::Gate(l, _) = op {
            let (c, flip) = *cells[l.cell].get_or_insert_with(|| {
                used += 1;
                (used - 1, l.negated)
            });
            Some(Lit {
                cell: c,
                negated: l.negated != flip,
            })
        } else {
            None
        };
        *slot = code(op, lit, &re);
    }
    key
}

fn perms(n: usize) -> Vec<Vec<usize>> {
    if n == 0 {
        return vec![vec![]];
    }
    let mut out = Vec::new();
    for p in perms(n - 1) {
        for at in 0..n {
            let mut q = p.clone();
            q.insert(at, n - 1);
            out.push(q);
        }
    }
    out
}

/// Whether `p` is the least encoding of its orbit. `node_perms` are those
/// that keep every node's kind, the only ones that can give a smaller key
/// when the kinds ascend.
fn is_canonical(p: &Program, node_perms: &[Vec<usize>], input_perms: &[Vec<usize>]) -> bool {
    let id: Vec<usize> = (0..p.nodes.len()).collect();
    let id_inputs: Vec<usize> = (0..input_perms[0].len()).collect();
    let raw = perm_key(p, &id, &id_inputs);
    // Already in normal form: cells by first use, merges sorted.
    let plain = {
        let mut key = [0; MAX_ENUM];
        for (k, op) in p.nodes.iter().enumerate() {
            let lit = if let Op::Gate(l, _) = op {
                Some(*l)
            } else {
                None
            };
            key[k] = code(op, lit, &|a| a);
        }
        key
    };
    if raw != plain
        || p.nodes
            .iter()
            .any(|op| matches!(op, Op::Merge(a, b) if src_code(*a) > src_code(*b)))
    {
        return false;
    }
    node_perms.iter().all(|pi| {
        input_perms
            .iter()
            .all(|sigma| perm_key(p, pi, sigma) >= raw)
    })
}

/// Every nondecreasing sequence of `n` kinds.
fn kind_seqs(n: usize, from: u32) -> Vec<Vec<u32>> {
    if n == 0 {
        return vec![vec![]];
    }
    (from..6)
        .flat_map(|k| {
            kind_seqs(n - 1, k).into_iter().map(move |mut rest| {
                rest.insert(0, k);
                rest
            })
        })
        .collect()
}

fn alphabet(kind: u32, inputs: usize, size: usize) -> Vec<Op> {
    let srcs: Vec<Src> = (0..inputs)
        .map(Src::Input)
        .chain((0..size).map(Src::Node))
        .collect();
    let mut out = Vec::new();
    for (i, &a) in srcs.iter().enumerate() {
        match kind {
            0 => out.extend(MAPS.iter().map(|&f| Op::Map(f, a))),
            1 => out.extend(PREDS.iter().map(|&p| Op::Filter(p, a))),
            2 => {
                for cell in 0..CELLS {
                    for negated in [false, true] {
                        out.push(Op::Gate(Lit { cell, negated }, a));
                    }
                }
            }
            3 => out.extend(srcs[i..].iter().map(|&b| Op::Merge(a, b))),
            4 => out.extend(srcs.iter().map(|&b| Op::OrElse(a, b))),
            _ => out.push(Op::HoldSteps(a)),
        }
    }
    out
}

/// Runs `work` on every canonical program of `size` nodes over `inputs`
/// inputs, on every core, each thread with its own accumulator.
fn par_canonical<A: Default + Send>(
    inputs: usize,
    size: usize,
    work: &(dyn Fn(&mut A, &Program) + Sync),
) -> Vec<A> {
    let alpha: Vec<Vec<Op>> = (0..6).map(|k| alphabet(k, inputs, size)).collect();
    let input_perms = perms(inputs);
    let all_node_perms = perms(size);
    // A task: a kind sequence and the first node's choice.
    let seqs = kind_seqs(size, 0);
    let tasks: Vec<(usize, usize)> = seqs
        .iter()
        .enumerate()
        .flat_map(|(si, seq)| (0..alpha[seq[0] as usize].len()).map(move |c| (si, c)))
        .collect();
    let next = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut acc = A::default();
                    loop {
                        let t = next.fetch_add(1, Ordering::Relaxed);
                        let Some(&(si, first)) = tasks.get(t) else {
                            break;
                        };
                        let seq = &seqs[si];
                        let node_perms: Vec<Vec<usize>> = all_node_perms
                            .iter()
                            .filter(|pi| (0..size).all(|k| seq[pi[k]] == seq[k]))
                            .cloned()
                            .collect();
                        let lens: Vec<usize> =
                            seq.iter().map(|&k| alpha[k as usize].len()).collect();
                        let mut idx = vec![0; size];
                        idx[0] = first;
                        loop {
                            let p = Program {
                                nodes: (0..size).map(|k| alpha[seq[k] as usize][idx[k]]).collect(),
                            };
                            if is_canonical(&p, &node_perms, &input_perms) {
                                work(&mut acc, &p);
                            }
                            // Next choice for nodes 1.., odometer style.
                            let mut k = size;
                            loop {
                                k -= 1;
                                if k == 0 {
                                    break;
                                }
                                idx[k] += 1;
                                if idx[k] < lens[k] {
                                    break;
                                }
                                idx[k] = 0;
                            }
                            if k == 0 {
                                break;
                            }
                        }
                    }
                    acc
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    })
}

/// Counts per size, input count and model. The class columns count
/// programs with at least one binding of that class; the `-b` ones count
/// (program, binding) pairs.
#[derive(Default, Clone, Copy)]
struct ERow {
    programs: u64,
    free: u64,
    /// Some cell states settle in every instant, some don't: only these
    /// can be constructive over reachable states and not free cells.
    mixed: u64,
    new: u64,
    dead_gate: u64,
    invariant: u64,
    dead_branch: u64,
    per_instant: u64,
    changing: u64,
    value_dep: u64,
    bindings: u64,
    new_b: u64,
    c_b: u64,
    changing_b: u64,
}

impl ERow {
    fn add(&mut self, o: &ERow) {
        self.programs += o.programs;
        self.free += o.free;
        self.mixed += o.mixed;
        self.new += o.new;
        self.dead_gate += o.dead_gate;
        self.invariant += o.invariant;
        self.dead_branch += o.dead_branch;
        self.per_instant += o.per_instant;
        self.changing += o.changing;
        self.value_dep += o.value_dep;
        self.bindings += o.bindings;
        self.new_b += o.new_b;
        self.c_b += o.c_b;
        self.changing_b += o.changing_b;
    }
}

/// (c) programs with a changing cell, grouped by their minimal form: the
/// nodes on or upstream of a cycle (`prune`), the least of its orbit.
struct Rec {
    cycle: Program,
    /// The smallest bound example: pruned as in the reachable-state mode,
    /// keeping what the cells read, then the least of its orbit.
    example: (usize, RecKey),
    program: Program,
    bind: Bind,
    /// Bit m: found under `MODELS[m]`.
    models: u8,
    /// Bit 0 (c1), bit 1 (c2).
    classes: u8,
    value_dep: bool,
    states: u8,
    /// Enumerated (program, binding) pairs in the group.
    pairs: u64,
    /// Smallest enumerated program size it came from.
    from_size: usize,
}

type RecKey = (Key, [u32; CELLS]);

#[derive(Default)]
struct Acc {
    rows: [ERow; 3],
    mismatches: u64,
    /// Free-cell constructive programs `judge` disagreed on.
    free_disagree: u64,
    sampled: u64,
    sampled_agree: u64,
    c_checked: u64,
    c_agree: u64,
    recs: BTreeMap<RecKey, Rec>,
}

struct Ctx {
    inputs: usize,
    size: usize,
    combos: Vec<Vec<Vec<V>>>,
    defs: Vec<CellDef>,
}

fn state_ok(p: &Program, combos: &[Vec<V>], s: u8) -> bool {
    combos.iter().all(|inp| {
        eval(p, inp, s, Filters::ByValue, false)
            .iter()
            .all(|v| v.settled())
    })
}

fn gate_cells(p: &Program) -> [bool; CELLS] {
    let mut used = [false; CELLS];
    for op in &p.nodes {
        if let Op::Gate(l, _) = op {
            used[l.cell] = true;
        }
    }
    used
}

fn analyze(ctx: &Ctx, acc: &mut Acc, p: &Program) {
    if !has_cycle(p, &|_, _| false) {
        return;
    }
    let used = gate_cells(p);
    let used_mask = (used[0] as u8) | (used[1] as u8) << 1;
    // ok[m][s]: every instant model m allows settles in cell state s. The
    // models' instants nest, one ⊂ >=1 ⊂ any, so each only refines the last.
    let mut ok = [[false; 4]; 3];
    for s in 0..4u8 {
        let rep = s & used_mask;
        if rep != s {
            for row in &mut ok {
                row[s as usize] = row[rep as usize];
            }
            continue;
        }
        ok[2][s as usize] = state_ok(p, &ctx.combos[2], s);
        ok[1][s as usize] = ok[2][s as usize] && state_ok(p, &ctx.combos[1], s);
        ok[0][s as usize] = ok[1][s as usize] && state_ok(p, &ctx.combos[0], s);
    }
    for (m, ok) in ok.iter().enumerate() {
        let row = &mut acc.rows[m];
        row.programs += 1;
        if ok.iter().all(|&o| o) {
            row.free += 1;
            let v = judge(p, &ctx.combos[m]);
            acc.mismatches += v.order_mismatches;
            if v.class.is_none() {
                acc.free_disagree += 1;
            }
            continue;
        }
        if ok.iter().all(|&o| !o) {
            continue;
        }
        row.mixed += 1;
        mixed(ctx, acc, p, m, ok, used);
    }
}

/// Tries every binding of a program some but not all of whose cell states
/// settle.
fn mixed(ctx: &Ctx, acc: &mut Acc, p: &Program, m: usize, ok: &[bool; 4], used: [bool; CELLS]) {
    let combos = &ctx.combos[m];
    let nsrc = ctx.inputs + ctx.size;
    let si = |a: Src| match a {
        Src::Input(i) => i,
        Src::Node(k) => ctx.inputs + k,
    };
    // Per settling state: for each pair of sources, which pairs of outcomes
    // (0 absent, 1 + v present with v) some instant gives, as a 25-bit set;
    // and the `or_else` nodes whose left side is present in every instant.
    let mut joint = vec![vec![0u32; nsrc * nsrc]; 4];
    let mut left_always = vec![vec![true; p.nodes.len()]; 4];
    for s in 0..4u8 {
        if !ok[s as usize] {
            continue;
        }
        let mut out = vec![0usize; nsrc];
        for inp in combos {
            let v = eval(p, inp, s, Filters::ByValue, false);
            let o = |x: V| match x {
                V::Present(x) => 1 + x as usize,
                _ => 0,
            };
            for i in 0..ctx.inputs {
                out[i] = o(inp[i]);
            }
            for k in 0..ctx.size {
                out[ctx.inputs + k] = o(v[k]);
            }
            for a in 0..nsrc {
                for b in 0..nsrc {
                    joint[s as usize][a * nsrc + b] |= 1 << (out[a] * 5 + out[b]);
                }
            }
            for (k, op) in p.nodes.iter().enumerate() {
                if let Op::OrElse(a, _) = op {
                    let left = match *a {
                        Src::Input(i) => inp[i],
                        Src::Node(j) => v[j],
                    };
                    if !left.present() {
                        left_always[s as usize][k] = false;
                    }
                }
            }
        }
    }
    let defs = |j: usize| -> &[CellDef] {
        if used[j] {
            &ctx.defs
        } else {
            std::slice::from_ref(&UNUSED)
        }
    };
    let mut memo: [Option<RClass>; 16] = [None; 16];
    // Which bindings the control samples: an FNV-style hash of the program
    // and the binding's position, so the sample doesn't depend on how the
    // threads split the work.
    let mut seen = p.nodes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, op| {
        (h ^ code(
            op,
            if let Op::Gate(l, _) = op {
                Some(*l)
            } else {
                None
            },
            &|a| a,
        ) as u64)
            .wrapping_mul(0x100_0000_01b3)
    }) ^ m as u64;
    let mut seen_class = [false; 4];
    let (mut any_new, mut changing, mut value_dep) = (false, false, false);
    for &d0 in defs(0) {
        for &d1 in defs(1) {
            let row = &mut acc.rows[m];
            row.bindings += 1;
            seen = seen.wrapping_mul(0x100_0000_01b3) ^ 1;
            let bind: Bind = [d0, d1];
            // Breadth-first over the cell states this binding reaches.
            let start = (used[0] && d0.init) as u8 | ((used[1] && d1.init) as u8) << 1;
            let mut reached = 1u8 << start;
            let mut stack = vec![start];
            let mut settles = true;
            while let Some(s) = stack.pop() {
                if !ok[s as usize] {
                    settles = false;
                    break;
                }
                let mut set = joint[s as usize][si(d0.src) * nsrc + si(d1.src)];
                while set != 0 {
                    let bit = set.trailing_zeros() as usize;
                    set &= set - 1;
                    let (oa, ob) = (bit / 5, bit % 5);
                    let mut next = s;
                    if used[0] && oa > 0 {
                        next = next & !1 | d0.pred.test(oa as u8 - 1) as u8;
                    }
                    if used[1] && ob > 0 {
                        next = next & !2 | (d1.pred.test(ob as u8 - 1) as u8) << 1;
                    }
                    if reached >> next & 1 == 0 {
                        reached |= 1 << next;
                        stack.push(next);
                    }
                }
            }
            let class = settles.then(|| {
                *memo[reached as usize].get_or_insert_with(|| {
                    let states: Vec<u8> = (0..4).filter(|s| reached >> s & 1 == 1).collect();
                    classify_reached(p, &states, &left_always)
                })
            });
            let is_c = matches!(class, Some(RClass::DeadBranch | RClass::PerInstant));
            if seen.is_multiple_of(SAMPLE_EVERY) && !is_c {
                acc.sampled += 1;
                if judge_reach(p, &bind, combos).class == class {
                    acc.sampled_agree += 1;
                }
            }
            let Some(class) = class else { continue };
            any_new = true;
            row.new_b += 1;
            let ci = match class {
                RClass::DeadGate => 0,
                RClass::Invariant => 1,
                RClass::DeadBranch => 2,
                RClass::PerInstant => 3,
            };
            seen_class[ci] = true;
            if !is_c {
                continue;
            }
            row.c_b += 1;
            let rv = judge_reach(p, &bind, combos);
            acc.c_checked += 1;
            acc.mismatches += rv.order_mismatches;
            if rv.class == Some(class) {
                acc.c_agree += 1;
            }
            value_dep |= rv.value_dependent;
            if !cells_change(p, &rv) {
                continue;
            }
            let (sp, sb) = prune_bound(p, &bind);
            acc.rows[m].changing_b += 1;
            changing = true;
            let (ekey, cp, cb) = canon_bound(&sp, &sb, ctx.inputs, true);
            let (key, cycle, _) = canon_bound(&prune(p), &[UNUSED; CELLS], ctx.inputs, false);
            let example = (cp.nodes.len(), ekey);
            let rec = acc.recs.entry(key).or_insert_with(|| Rec {
                cycle,
                example,
                program: cp.clone(),
                bind: cb,
                models: 0,
                classes: 0,
                value_dep: false,
                states: rv.states,
                pairs: 0,
                from_size: ctx.size,
            });
            rec.models |= 1 << m;
            rec.classes |= 1 << (ci - 2);
            rec.value_dep |= rv.value_dependent;
            rec.pairs += 1;
            rec.from_size = rec.from_size.min(ctx.size);
            if example < rec.example {
                rec.example = example;
                rec.program = cp;
                rec.bind = cb;
                rec.states = rv.states;
            }
        }
    }
    let row = &mut acc.rows[m];
    if any_new {
        row.new += 1;
    }
    row.dead_gate += seen_class[0] as u64;
    row.invariant += seen_class[1] as u64;
    row.dead_branch += seen_class[2] as u64;
    row.per_instant += seen_class[3] as u64;
    row.changing += changing as u64;
    row.value_dep += value_dep as u64;
}

/// Whether some gate on or upstream of a cycle is open in some reachable
/// state and closed in another: a cell the cycle depends on changes. This is
/// the reachable-state mode's `fix` definition as it reads; its census asks
/// it of `prune_bound`'s program, which also keeps gates that only feed a
/// cell, so a cell that changes but gates nothing the cycle reads would
/// count there.
fn cells_change(p: &Program, rv: &RVerdict) -> bool {
    let mut keep = on_cycle(p);
    keep_upstream(p, &mut keep);
    let reached: Vec<u8> = (0..1u8 << CELLS)
        .filter(|s| rv.states >> s & 1 == 1)
        .collect();
    p.nodes.iter().zip(&keep).any(|(op, &kept)| match op {
        Op::Gate(lit, _) if kept => {
            let open = reached.iter().filter(|&&s| lit.open(s)).count();
            open != 0 && open != reached.len()
        }
        _ => false,
    })
}

/// The least form of a bound program under every symmetry: node and input
/// permutations, swapping the cells, negating either.
/// With `with_bind` false the binding is ignored: the program's own orbit.
fn canon_bound(
    p: &Program,
    bind: &Bind,
    inputs: usize,
    with_bind: bool,
) -> (RecKey, Program, Bind) {
    let n = p.nodes.len();
    let used = gate_cells(p);
    let mut best: Option<(RecKey, Program, Bind)> = None;
    for pi in perms(n) {
        for sigma in perms(inputs) {
            for swap in [false, true] {
                for neg in 0..4u8 {
                    let re = |a: Src| match a {
                        Src::Input(i) => Src::Input(sigma[i]),
                        Src::Node(k) => Src::Node(pi[k]),
                    };
                    let cell = |c: usize| if swap { 1 - c } else { c };
                    let flip = |c: usize| neg >> c & 1 == 1;
                    let mut nodes = vec![Op::HoldSteps(Src::Input(0)); n];
                    for (k, op) in p.nodes.iter().enumerate() {
                        nodes[pi[k]] = match *op {
                            Op::Map(f, a) => Op::Map(f, re(a)),
                            Op::Filter(pr, a) => Op::Filter(pr, re(a)),
                            Op::Gate(l, a) => Op::Gate(
                                Lit {
                                    cell: cell(l.cell),
                                    negated: l.negated != flip(l.cell),
                                },
                                re(a),
                            ),
                            Op::Merge(a, b) => {
                                let (x, y) = (re(a), re(b));
                                if src_code(x) <= src_code(y) {
                                    Op::Merge(x, y)
                                } else {
                                    Op::Merge(y, x)
                                }
                            }
                            Op::OrElse(a, b) => Op::OrElse(re(a), re(b)),
                            Op::HoldSteps(a) => Op::HoldSteps(re(a)),
                        };
                    }
                    let mut b = [UNUSED; CELLS];
                    for c in 0..CELLS {
                        if used[c] {
                            let d = bind[c];
                            b[cell(c)] = CellDef {
                                src: re(d.src),
                                pred: if flip(c) { complement(d.pred) } else { d.pred },
                                init: d.init != flip(c),
                            };
                        }
                    }
                    let q = Program { nodes };
                    let mut key = [0; MAX_ENUM];
                    for (k, op) in q.nodes.iter().enumerate() {
                        let lit = if let Op::Gate(l, _) = op {
                            Some(*l)
                        } else {
                            None
                        };
                        key[k] = code(op, lit, &|a| a);
                    }
                    let bkey = if with_bind {
                        b.map(|d| src_code(d.src) << 4 | pred_index(d.pred) << 1 | d.init as u32)
                    } else {
                        [0; CELLS]
                    };
                    let k = (key, bkey);
                    if best.as_ref().is_none_or(|(k0, _, _)| k < *k0) {
                        best = Some((k, q, b));
                    }
                }
            }
        }
    }
    best.unwrap()
}

/// Control: the canonical enumeration finds exactly one program per orbit,
/// checked against the least key of every raw program.
fn check_enumerator(inputs: usize, size: usize) -> (u64, u64) {
    let found: u64 = par_canonical::<u64>(inputs, size, &|n, _| *n += 1)
        .iter()
        .sum();
    let pis = perms(size);
    let sigmas = perms(inputs);
    let srcs: Vec<Src> = (0..inputs)
        .map(Src::Input)
        .chain((0..size).map(Src::Node))
        .collect();
    let mut all: Vec<Op> = (0..6).flat_map(|k| alphabet(k, inputs, size)).collect();
    // The raw alphabet has merges both ways round.
    for &a in &srcs {
        for &b in &srcs {
            if src_code(a) > src_code(b) {
                all.push(Op::Merge(a, b));
            }
        }
    }
    let mut orbits = std::collections::HashSet::new();
    let mut idx = vec![0; size];
    loop {
        let p = Program {
            nodes: idx.iter().map(|&i| all[i]).collect(),
        };
        let least = pis
            .iter()
            .flat_map(|pi| sigmas.iter().map(move |s| (pi, s)))
            .map(|(pi, s)| perm_key(&p, pi, s))
            .min()
            .unwrap();
        orbits.insert(least);
        let mut k = size;
        loop {
            if k == 0 {
                return (found, orbits.len() as u64);
            }
            k -= 1;
            idx[k] += 1;
            if idx[k] < all.len() {
                break;
            }
            idx[k] = 0;
        }
    }
}

fn line(label: &str, r: &ERow) -> String {
    format!(
        "{:>4} {:>9} {:>6} {:>8} {:>7} {:>6} {:>6} {:>5} {:>5} {:>4} {:>4} {:>11} {:>9} {:>6} {:>6}",
        label,
        r.programs,
        r.free,
        r.mixed,
        r.new,
        r.dead_gate,
        r.invariant,
        r.dead_branch,
        r.per_instant,
        r.changing,
        r.value_dep,
        r.bindings,
        r.new_b,
        r.c_b,
        r.changing_b
    )
}

pub(super) fn run() {
    println!(
        "ternary-loop-census --enumerate: every program of {MIN_SIZE} to {MAX_ENUM} nodes, every cell binding\n"
    );
    println!(
        "Every program of {MIN_SIZE} to {MAX_ENUM} nodes over the reachable-state mode's subset that has a"
    );
    println!("same-instant cycle, counted once per orbit under renaming nodes, permuting");
    println!("inputs, swapping the two cells and negating a cell. Each cell a gate reads is");
    println!("`c = src.map(pred).hold(init)`, and every binding is tried: src any input or");
    println!("stream, pred one of four, init false or true, per cell. Classes as in the");
    println!("reachable-state mode.");
    println!("free: constructive over free cells. mixed: some cell states settle in every");
    println!("instant and some don't; only these can be new. new: some binding makes it");
    println!("constructive over reachable states. (a'd) (a'i) (c1) (c2): some binding of");
    println!("that class. chg: some (c) binding with a changing cell: a gate on or upstream");
    println!("of a cycle is open in one reachable state and closed in another. val: some (c)");
    println!(
        "binding that is value-dependent. bindings, new-b, (c)-b, chg-b: (program, binding) pairs"
    );
    println!("over the mixed programs.\n");

    let mut recs: Vec<(usize, BTreeMap<RecKey, Rec>)> = Vec::new();
    let mut totals_by_inputs = Vec::new();
    let (mut mismatches, mut free_disagree, mut sampled, mut sampled_agree) = (0, 0, 0, 0);
    let (mut c_checked, mut c_agree) = (0, 0);
    for inputs in [2, 3] {
        let mut rows = Vec::new();
        let mut merged: BTreeMap<RecKey, Rec> = BTreeMap::new();
        for size in MIN_SIZE..=MAX_ENUM {
            let start = std::time::Instant::now();
            let ctx = Ctx {
                inputs,
                size,
                combos: MODELS.iter().map(|&m| input_combos(inputs, m)).collect(),
                defs: (0..inputs)
                    .map(Src::Input)
                    .chain((0..size).map(Src::Node))
                    .flat_map(|src| {
                        PREDS.iter().flat_map(move |&pred| {
                            [false, true].map(|init| CellDef { src, pred, init })
                        })
                    })
                    .collect(),
            };
            let accs = par_canonical::<Acc>(inputs, size, &|acc, p| analyze(&ctx, acc, p));
            let mut r = [ERow::default(); 3];
            for a in accs {
                for (row, add) in r.iter_mut().zip(&a.rows) {
                    row.add(add);
                }
                mismatches += a.mismatches;
                free_disagree += a.free_disagree;
                sampled += a.sampled;
                sampled_agree += a.sampled_agree;
                c_checked += a.c_checked;
                c_agree += a.c_agree;
                for (k, rec) in a.recs {
                    match merged.get_mut(&k) {
                        Some(e) => {
                            e.models |= rec.models;
                            e.classes |= rec.classes;
                            e.value_dep |= rec.value_dep;
                            e.pairs += rec.pairs;
                            e.from_size = e.from_size.min(rec.from_size);
                            if rec.example < e.example {
                                e.example = rec.example;
                                e.program = rec.program;
                                e.bind = rec.bind;
                                e.states = rec.states;
                            }
                        }
                        None => {
                            merged.insert(k, rec);
                        }
                    }
                }
            }
            // Progress only: timings aren't results on a shared machine.
            eprintln!("inputs {inputs} size {size}: {:.1?}", start.elapsed());
            rows.push(r);
        }
        let mut totals = [ERow::default(); 3];
        for (m, &model) in MODELS.iter().enumerate() {
            println!(
                "{inputs} inputs, model `{}`: {}",
                model.name(),
                model.describe()
            );
            println!(
                "{:>4} {:>9} {:>6} {:>8} {:>7} {:>6} {:>6} {:>5} {:>5} {:>4} {:>4} {:>11} {:>9} {:>6} {:>6}",
                "size",
                "programs",
                "free",
                "mixed",
                "new",
                "(a'd)",
                "(a'i)",
                "(c1)",
                "(c2)",
                "chg",
                "val",
                "bindings",
                "new-b",
                "(c)-b",
                "chg-b"
            );
            for (i, r) in rows.iter().enumerate() {
                totals[m].add(&r[m]);
                println!("{}", line(&(i + MIN_SIZE).to_string(), &r[m]));
            }
            println!("{}\n", line("all", &totals[m]));
        }
        totals_by_inputs.push((inputs, totals));
        recs.push((inputs, merged));
    }

    for (inputs, size) in [(1, 2), (1, 3), (2, 2), (2, 3), (3, 2)] {
        let (found, orbits) = check_enumerator(inputs, size);
        println!(
            "control: {inputs} inputs, size {size}: enumerator found {found} programs, brute force {orbits} orbits"
        );
    }
    println!("control: {free_disagree} free-constructive programs `judge` refused");
    println!(
        "control: `judge_reach` agreed on {sampled_agree} of {sampled} sampled non-(c) bindings"
    );
    println!("control: `judge_reach` agreed on {c_agree} of {c_checked} (c) bindings");
    println!("control: {mismatches} instants where forward and reverse sweeps disagreed\n");

    println!("Every (c) program with a changing cell, grouped by minimal form: the nodes");
    println!("on or upstream of a cycle, up to symmetry. bindings: enumerated (program,");
    println!("binding) pairs in the group. Then the smallest example with its cells, pruned");
    println!("as in the reachable-state mode; reachable states listed as c1c0.\n");
    for (inputs, merged) in &recs {
        if merged.is_empty() {
            println!("{inputs} inputs: none found\n");
            continue;
        }
        for (i, rec) in merged.values().enumerate() {
            let models: Vec<&str> = (0..3)
                .filter(|m| rec.models >> m & 1 == 1)
                .map(|m| MODELS[m].name())
                .collect();
            let classes: Vec<&str> = [(1, "(c1)"), (2, "(c2)")]
                .iter()
                .filter(|(b, _)| rec.classes & b != 0)
                .map(|(_, s)| *s)
                .collect();
            // The same example with one more input, which may fire alone.
            let m = (0..3).find(|m| rec.models >> m & 1 == 1).unwrap();
            let wider = input_combos(inputs + 1, MODELS[m]);
            let survives = judge_reach(&rec.program, &rec.bind, &wider).class.is_some();
            println!(
                "{inputs} inputs, #{}: {} under {}; value-dependent: {}; bindings: {}; from size {}; example with a {} input under `{}`: {}:",
                i + 1,
                classes.join(" "),
                models.join(", "),
                if rec.value_dep { "yes" } else { "no" },
                rec.pairs,
                rec.from_size,
                ["", "2nd", "3rd", "4th"][*inputs],
                MODELS[m].name(),
                if survives {
                    "constructive"
                } else {
                    "not constructive"
                }
            );
            println!("{}", show(&rec.cycle));
            println!(
                "  smallest example (reachable: {}):",
                states_text(rec.states)
            );
            println!("{}", show_bound(&rec.program, &rec.bind));
        }
    }

    for (inputs, t) in &totals_by_inputs {
        let c_any: Vec<String> = (0..3)
            .map(|m| {
                format!(
                    "`{}` {} programs new, (c1) {}, (c2) {}, {} with a changing cell",
                    MODELS[m].name(),
                    t[m].new,
                    t[m].dead_branch,
                    t[m].per_instant,
                    t[m].changing
                )
            })
            .collect();
        println!(
            "verdict ({inputs} inputs, {} refused programs of {MIN_SIZE}-{MAX_ENUM} nodes, every binding): {}; {} minimal forms of (c) with a changing cell.",
            t[0].programs,
            c_any.join("; "),
            recs.iter()
                .find(|(i, _)| i == inputs)
                .map_or(0, |(_, r)| r.len())
        );
    }
}
