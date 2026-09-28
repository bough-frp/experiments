//! Programs as data, the three variants' builders, and the random
//! generator.
//!
//! A program is a list of ops over earlier refs, so it is loop-free by
//! construction. `construct` carries a body, another op list, that runs at
//! each event's time with the event's value as a constant cell and every
//! ref of the enclosing scope captured. Every node a run builds is recorded
//! with its creation time, its op and its argument values, so the check can
//! rebuild the same primitive over other arguments.

use std::cell::{Cell as StdCell, RefCell};
use std::fmt::Write as _;
use std::rc::Rc;

use crate::model::{F1, F2, Fan, Kind, Node, P, S, T, V, node};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Variant {
    /// The equations as written: no creation time on `Split`, `SwitchC`
    /// scanning its whole outer, `Split` unsorted.
    Text,
    /// Option (a): every primitive built at t0 sees its arguments from t0 on.
    CutAll,
    /// Option (b): only state-holders and time-movers are cut (`Hold` and
    /// `Value` as in the text, plus `SwitchC`'s outer and `Split`'s input).
    CutStateful,
}

impl Variant {
    pub const ALL: [Variant; 3] = [Variant::Text, Variant::CutAll, Variant::CutStateful];

    pub fn name(self) -> &'static str {
        match self {
            Variant::Text => "text",
            Variant::CutAll => "cut-all",
            Variant::CutStateful => "cut-stateful",
        }
    }
}

/// Stream or cell of Int, stream or cell of cells, cell of streams.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ty {
    SI,
    CI,
    SC,
    CC,
    CS,
}

#[derive(Clone)]
pub enum Op {
    Input(usize),
    Never,
    Constant(i64),
    Map(usize, F1),
    Filter(usize, P),
    Merge(usize, usize, F2),
    Snapshot(usize, usize, F2),
    Split(usize, Fan),
    Updates(usize),
    Value(usize),
    Hold(usize, i64),
    MapC(usize, F1),
    Lift2(usize, usize, F2),
    /// A selector cell and its choices, all `CI` (a `CC`) or all `SI` (a `CS`).
    Pick(usize, Vec<usize>),
    SwitchCell(usize),
    SwitchStream(usize),
    Construct(usize, Rc<BodyR>),
    /// Hold of a stream of cells, with an initial cell.
    HoldC(usize, usize),
}

/// A `construct` body: its ops, and the ref of the cell it returns. Its
/// scope is the enclosing scope, then the event's value, then its ops.
pub struct BodyR {
    pub ops: Vec<Op>,
    pub ret: usize,
}

pub struct Prog {
    pub ops: Vec<Op>,
}

impl Op {
    pub fn refs(&self) -> Vec<usize> {
        match self {
            Op::Input(_) | Op::Never | Op::Constant(_) => vec![],
            Op::Map(a, _)
            | Op::Filter(a, _)
            | Op::Split(a, _)
            | Op::Updates(a)
            | Op::Value(a)
            | Op::Hold(a, _)
            | Op::MapC(a, _)
            | Op::SwitchCell(a)
            | Op::SwitchStream(a)
            | Op::Construct(a, _) => vec![*a],
            Op::Merge(a, b, _) | Op::Snapshot(a, b, _) | Op::Lift2(a, b, _) | Op::HoldC(a, b) => {
                vec![*a, *b]
            }
            Op::Pick(a, cs) => std::iter::once(*a).chain(cs.iter().copied()).collect(),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Op::Input(_) => "input",
            Op::Never => "never",
            Op::Constant(_) => "constant",
            Op::Map(..) => "map",
            Op::Filter(..) => "filter",
            Op::Merge(..) => "merge",
            Op::Snapshot(..) => "snapshot",
            Op::Split(_, Fan::One) => "defer",
            Op::Split(..) => "split",
            Op::Updates(_) => "updates",
            Op::Value(_) => "value",
            Op::Hold(..) => "hold",
            Op::MapC(..) => "map_cell",
            Op::Lift2(..) => "lift2",
            Op::Pick(..) => "pick",
            Op::SwitchCell(_) => "switch_cell",
            Op::SwitchStream(_) => "switch_stream",
            Op::Construct(..) => "construct",
            Op::HoldC(..) => "hold_cell",
        }
    }

    /// The op's result type, given its arguments' types.
    pub fn ty(&self, tys: &[Ty]) -> Ty {
        match self {
            Op::Input(_)
            | Op::Never
            | Op::Map(..)
            | Op::Filter(..)
            | Op::Merge(..)
            | Op::Snapshot(..)
            | Op::Split(..)
            | Op::Updates(_)
            | Op::Value(_)
            | Op::SwitchStream(_) => Ty::SI,
            Op::Constant(_) | Op::Hold(..) | Op::MapC(..) | Op::Lift2(..) | Op::SwitchCell(_) => {
                Ty::CI
            }
            Op::Pick(_, cs) if tys[cs[0]] == Ty::SI => Ty::CS,
            Op::Pick(..) | Op::HoldC(..) => Ty::CC,
            Op::Construct(..) => Ty::SC,
        }
    }
}

/// Where a node was built: the chain of `construct` instances (the
/// construct's op index and the event time) and its own op index. Stable
/// across variants and histories, so runs can be matched node by node.
pub type Id = (Vec<(usize, T)>, usize);

/// One node a run built.
pub struct Reg {
    pub id: Id,
    pub t0: T,
    pub op: Op,
    pub args: Vec<V>,
    pub arg_tys: Vec<Ty>,
    pub scope: Rc<Scope>,
    pub out: V,
}

/// A scope: its values and, in parallel, their types from the recipe.
#[derive(Clone, Default)]
pub struct Scope {
    pub vals: Vec<V>,
    pub tys: Vec<Ty>,
}

pub struct Ctx {
    pub variant: Variant,
    pub inputs: Vec<S>,
    pub reg: RefCell<Vec<Reg>>,
    /// Off while the check rebuilds nodes over perturbed arguments.
    pub recording: StdCell<bool>,
}

/// Builds one primitive at t0 over the given arguments, with the cuts its
/// variant makes. The recipe's own refs are ignored: `args` are the
/// arguments, in `refs()` order.
pub fn make(
    ctx: &Rc<Ctx>,
    op: &Op,
    args: &[V],
    t0: &T,
    scope: &Rc<Scope>,
    path: &[(usize, T)],
    idx: usize,
) -> V {
    let v = ctx.variant;
    let cut = |a: &V| -> Rc<Node> {
        let n = node(a).clone();
        if n.is_cell() {
            Node::new(Kind::CutC(n, t0.clone()))
        } else {
            Node::new(Kind::CutS(n, t0.clone()))
        }
    };
    // An argument edge: cut under option (a) only.
    let w = |i: usize| -> Rc<Node> {
        if v == Variant::CutAll {
            cut(&args[i])
        } else {
            node(&args[i]).clone()
        }
    };
    let kind = match op {
        Op::Input(i) => Kind::Mk(ctx.inputs[*i].clone()),
        Op::Never => Kind::Never,
        Op::Constant(k) => Kind::Constant(V::I(*k)),
        Op::Map(_, f) => Kind::Map(*f, w(0)),
        Op::Filter(_, p) => Kind::Filter(*p, w(0)),
        Op::Merge(_, _, f) => Kind::Merge(w(0), w(1), *f),
        Op::Snapshot(_, _, f) => Kind::Snapshot(*f, w(0), w(1)),
        Op::Split(_, fan) => match v {
            Variant::Text => Kind::Split(node(&args[0]).clone(), *fan, false),
            _ => Kind::Split(cut(&args[0]), *fan, true),
        },
        Op::Updates(_) => Kind::Updates(w(0)),
        Op::Value(_) => Kind::Value(w(0), t0.clone()),
        Op::Hold(_, init) => Kind::Hold(V::I(*init), w(0), t0.clone()),
        Op::MapC(_, f) => Kind::MapC(*f, w(0)),
        Op::Lift2(_, _, f) => Kind::Apply2(*f, w(0), w(1)),
        Op::Pick(..) => Kind::Pick(w(0), args[1..].to_vec()),
        Op::SwitchCell(_) => match v {
            Variant::Text => Kind::SwitchC(node(&args[0]).clone(), t0.clone()),
            _ => Kind::SwitchC(cut(&args[0]), t0.clone()),
        },
        Op::SwitchStream(_) => Kind::SwitchS(w(0)),
        Op::HoldC(..) => Kind::Hold(args[1].clone(), w(0), t0.clone()),
        Op::Construct(_, body) => {
            let (ctx2, body2, scope2) = (ctx.clone(), body.clone(), scope.clone());
            let mut base = path.to_vec();
            base.push((idx, T::new()));
            let f = move |t: &T, a: &V| -> V {
                let mut p = base.clone();
                p.last_mut().unwrap().1 = t.clone();
                let mut s = (*scope2).clone();
                s.vals.push(V::N(Node::new(Kind::Constant(a.clone()))));
                s.tys.push(Ty::CI);
                let s = build_scope(&ctx2, &body2.ops, s, t, &p);
                s.vals[body2.ret].clone()
            };
            Kind::Execute(w(0), Rc::new(f))
        }
    };
    V::N(Node::new(kind))
}

/// Builds a scope's ops at t0, recording each node.
fn build_scope(ctx: &Rc<Ctx>, ops: &[Op], mut scope: Scope, t0: &T, path: &[(usize, T)]) -> Scope {
    for (i, op) in ops.iter().enumerate() {
        let refs = op.refs();
        let args: Vec<V> = refs.iter().map(|&r| scope.vals[r].clone()).collect();
        let arg_tys: Vec<Ty> = refs.iter().map(|&r| scope.tys[r]).collect();
        let ty = op.ty(&scope.tys);
        let sc = Rc::new(scope.clone());
        let out = make(ctx, op, &args, t0, &sc, path, i);
        if ctx.recording.get() {
            ctx.reg.borrow_mut().push(Reg {
                id: (path.to_vec(), i),
                t0: t0.clone(),
                op: op.clone(),
                args,
                arg_tys,
                scope: sc,
                out: out.clone(),
            });
        }
        scope.vals.push(out);
        scope.tys.push(ty);
    }
    scope
}

/// Runs a program on input histories under a variant, and forces every
/// node, including every node a body builds. Returns the context, whose
/// registry lists every node built. The caller must `finish` it.
pub fn run(prog: &Prog, inputs: &[S], variant: Variant) -> Rc<Ctx> {
    let ctx = Rc::new(Ctx {
        variant,
        inputs: inputs.to_vec(),
        reg: RefCell::new(Vec::new()),
        recording: StdCell::new(true),
    });
    build_scope(&ctx, &prog.ops, Scope::default(), &vec![0], &[]);
    // Forcing a construct runs its bodies, which records more nodes.
    let mut i = 0;
    while i < ctx.reg.borrow().len() {
        let out = ctx.reg.borrow()[i].out.clone();
        node(&out).den();
        i += 1;
    }
    ctx.recording.set(false);
    ctx
}

/// Breaks the context's reference cycle (bodies capture it).
pub fn finish(ctx: &Rc<Ctx>) {
    ctx.reg.borrow_mut().clear();
}

// ---------------------------------------------------------------------------
// Random generation.

/// SplitMix64.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    pub fn chance(&mut self, p: f64) -> bool {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64 <= p
    }

    pub fn int(&mut self, lo: i64, hi: i64) -> i64 {
        lo + self.below((hi - lo + 1) as usize) as i64
    }
}

pub const INPUTS: usize = 2;
pub const TXS: i32 = 4;

/// Input histories: each input fires at external times `[1]..=[TXS]`.
pub fn gen_inputs(rng: &mut Rng) -> Vec<S> {
    (0..INPUTS)
        .map(|_| {
            (1..=TXS)
                .filter(|_| rng.chance(0.6))
                .collect::<Vec<_>>()
                .into_iter()
                .map(|k| (vec![k], V::I(rng.int(0, 9))))
                .collect()
        })
        .collect()
}

fn pick_of(rng: &mut Rng, tys: &[Ty], ty: Ty) -> Option<usize> {
    let c: Vec<usize> = (0..tys.len()).filter(|&i| tys[i] == ty).collect();
    (!c.is_empty()).then(|| c[rng.below(c.len())])
}

fn f1(rng: &mut Rng) -> F1 {
    if rng.chance(0.5) {
        F1::Add(rng.int(1, 5))
    } else {
        F1::Mul(rng.int(2, 3))
    }
}

fn f2(rng: &mut Rng) -> F2 {
    [F2::Add, F2::Sub, F2::Left, F2::Right][rng.below(4)]
}

/// One random op over the scope's types, or `None` if the draw needs a type
/// the scope lacks.
fn gen_op(rng: &mut Rng, tys: &[Ty], depth: usize) -> Option<Op> {
    // Weights favour the shapes the claim is about: splits, defers and
    // switches, and constructs that build them at child instants.
    const TABLE: [(u32, u8); 19] = [
        (2, 0),  // map
        (1, 1),  // filter
        (2, 2),  // merge
        (2, 3),  // snapshot
        (2, 4),  // split
        (3, 5),  // defer
        (1, 6),  // updates
        (1, 7),  // value
        (3, 8),  // hold
        (1, 9),  // map_cell
        (1, 10), // lift2
        (2, 11), // pick cells
        (1, 12), // pick streams
        (3, 13), // switch_cell
        (2, 14), // switch_stream
        (3, 15), // construct
        (3, 16), // hold_cell
        (1, 17), // constant
        (1, 18), // never
    ];
    let total: u32 = TABLE.iter().map(|x| x.0).sum();
    let mut r = rng.below(total as usize) as u32;
    let mut which = 0;
    for (w, k) in TABLE {
        if r < w {
            which = k;
            break;
        }
        r -= w;
    }
    let si = |rng: &mut Rng| pick_of(rng, tys, Ty::SI);
    let ci = |rng: &mut Rng| pick_of(rng, tys, Ty::CI);
    Some(match which {
        0 => Op::Map(si(rng)?, f1(rng)),
        1 => Op::Filter(
            si(rng)?,
            if rng.chance(0.5) {
                P::Even
            } else {
                P::Gt(rng.int(0, 20))
            },
        ),
        2 => Op::Merge(si(rng)?, si(rng)?, f2(rng)),
        3 => Op::Snapshot(si(rng)?, ci(rng)?, f2(rng)),
        4 => Op::Split(si(rng)?, Fan::Spread(rng.int(2, 3) as u8)),
        5 => Op::Split(si(rng)?, Fan::One),
        6 => Op::Updates(ci(rng)?),
        7 => Op::Value(ci(rng)?),
        8 => Op::Hold(si(rng)?, rng.int(0, 9)),
        9 => Op::MapC(ci(rng)?, f1(rng)),
        10 => Op::Lift2(ci(rng)?, ci(rng)?, f2(rng)),
        11 | 12 => {
            let sel = ci(rng)?;
            let n = rng.int(2, 3) as usize;
            let mut cs = Vec::new();
            for _ in 0..n {
                cs.push(if which == 11 { ci(rng)? } else { si(rng)? });
            }
            Op::Pick(sel, cs)
        }
        13 => Op::SwitchCell(pick_of(rng, tys, Ty::CC)?),
        14 => Op::SwitchStream(pick_of(rng, tys, Ty::CS)?),
        15 if depth < 2 => {
            let s = si(rng)?;
            Op::Construct(s, Rc::new(gen_body(rng, tys, depth + 1)))
        }
        16 => Op::HoldC(pick_of(rng, tys, Ty::SC)?, ci(rng)?),
        17 => Op::Constant(rng.int(0, 9)),
        18 => Op::Never,
        _ => return None,
    })
}

fn push(ops: &mut Vec<Op>, tys: &mut Vec<Ty>, op: Op) {
    let t = op.ty(tys);
    ops.push(op);
    tys.push(t);
}

/// Adds random ops, and after each `construct` usually the hold and switch
/// that observe it, as a program would.
fn fill(rng: &mut Rng, ops: &mut Vec<Op>, tys: &mut Vec<Ty>, n: usize, depth: usize) {
    let mut added = 0;
    while added < n {
        let Some(op) = gen_op(rng, tys, depth) else {
            continue;
        };
        let is_construct = matches!(op, Op::Construct(..));
        push(ops, tys, op);
        added += 1;
        if is_construct && rng.chance(0.8) {
            let sc = tys.len() - 1;
            let init = pick_of(rng, tys, Ty::CI).unwrap();
            push(ops, tys, Op::HoldC(sc, init));
            push(ops, tys, Op::SwitchCell(tys.len() - 1));
        }
    }
}

fn gen_body(rng: &mut Rng, outer: &[Ty], depth: usize) -> BodyR {
    let mut tys = outer.to_vec();
    tys.push(Ty::CI); // the event's value
    let base = tys.len();
    let mut ops = Vec::new();
    // A late switch of an older outer, when there is one: the F6 shape.
    if let Some(cc) = pick_of(rng, &tys[..base - 1], Ty::CC)
        && rng.chance(0.5)
    {
        push(&mut ops, &mut tys, Op::SwitchCell(cc));
    }
    let n = rng.int(2, 5) as usize;
    fill(rng, &mut ops, &mut tys, n, depth);
    // Return a cell the body built if it built one, else the event's value.
    let own: Vec<usize> = (base..tys.len()).filter(|&i| tys[i] == Ty::CI).collect();
    let ret = if own.is_empty() {
        base - 1
    } else {
        own[rng.below(own.len())]
    };
    BodyR { ops, ret }
}

pub fn gen_prog(rng: &mut Rng) -> Prog {
    let mut ops = Vec::new();
    let mut tys = Vec::new();
    for i in 0..INPUTS {
        push(&mut ops, &mut tys, Op::Input(i));
    }
    // A defer, so there are child instants to construct at.
    push(&mut ops, &mut tys, Op::Split(rng.below(INPUTS), Fan::One));
    push(&mut ops, &mut tys, Op::Hold(rng.below(INPUTS), 0));
    let n = rng.int(3, 8) as usize;
    fill(rng, &mut ops, &mut tys, n, 0);
    Prog { ops }
}

/// The refs of the enclosing scope a body reads, nested bodies included:
/// what a `construct` depends on besides its stream. `base` is the scope's
/// length at the construct, which is also the event's ref.
pub fn captured(b: &BodyR, base: usize, out: &mut Vec<usize>) {
    for (i, op) in b.ops.iter().enumerate() {
        out.extend(op.refs().into_iter().filter(|&r| r < base));
        if let Op::Construct(_, inner) = op {
            let mut deeper = Vec::new();
            captured(inner, base + 1 + i, &mut deeper);
            out.extend(deeper.into_iter().filter(|&r| r < base));
        }
    }
    if b.ret < base {
        out.push(b.ret);
    }
    out.sort();
    out.dedup();
}

/// Every op in the program, bodies included.
pub fn size(ops: &[Op]) -> usize {
    ops.iter()
        .map(|op| match op {
            Op::Construct(_, b) => 1 + size(&b.ops),
            _ => 1,
        })
        .sum()
}

/// The program as text: `rN = op(args)`, bodies indented.
pub fn show_prog(ops: &[Op], base: usize, indent: usize, out: &mut String) {
    for (i, op) in ops.iter().enumerate() {
        let pad = " ".repeat(indent);
        let r = |x: &usize| format!("r{x}");
        let args: Vec<String> = op.refs().iter().map(r).collect();
        let extra = match op {
            Op::Input(k) => format!("{k}"),
            Op::Constant(k) => format!("{k}"),
            Op::Map(_, f) | Op::MapC(_, f) => format!(", {f:?}"),
            Op::Filter(_, p) => format!(", {p:?}"),
            Op::Merge(_, _, f) | Op::Snapshot(_, _, f) | Op::Lift2(_, _, f) => format!(", {f:?}"),
            Op::Split(_, Fan::Spread(k)) => format!(", {k}"),
            Op::Hold(_, k) => format!(", {k}"),
            _ => String::new(),
        };
        let _ = write!(
            out,
            "{pad}r{} = {}({}{extra})",
            base + i,
            op.name(),
            args.join(", ")
        );
        if let Op::Construct(_, b) = op {
            // The body's scope is the enclosing one up to the construct,
            // then the event, which takes the construct's own number.
            let _ = writeln!(out, ", body at each event, r{} = its value:", base + i);
            show_prog(&b.ops, base + i + 1, indent + 4, out);
            let _ = writeln!(out, "{pad}    returns r{}", b.ret);
        } else {
            let _ = writeln!(out);
        }
    }
}
