//! The denotation: App. E's loop-free equations over finite `[Int]`-timed
//! lists, transcribed from `Reactive/Sodium/Denotational.hs` (revision 1.1)
//! clause by clause, so that the `text` variant is the text as written.
//!
//! Two wrappers carry every cut the other variants make: `CutS` keeps a
//! stream's events at or after t0, and `CutC` is the drafts' chopped cell,
//! `concrete (chopFront (steps c) t0)`. A variant is then only a choice of
//! which argument edges get a wrapper (see `prog::make`), which keeps the
//! equations themselves shared by all three.
//!
//! Values are dynamically typed: an `Int`, or a node (a stream or a cell).
//! That is enough for cells of cells, cells of streams and streams of cells,
//! which is all the higher-order shapes `switch_cell`, `switch_stream` and
//! `construct` need.

use std::cell::OnceCell;
use std::fmt;
use std::rc::Rc;

/// Hierarchical time. `Vec`'s `Ord` is Haskell's list order: prefix first,
/// then lexicographic, so `[1] < [1,0] < [1,1] < [2]`.
pub type T = Vec<i32>;

#[derive(Clone)]
pub enum V {
    I(i64),
    N(Rc<Node>),
}

/// `S a` and the step list of `C a`. The text documents both "for
/// increasing T values"; the equations don't enforce it, and `text` breaks
/// it (F6, F7), so nothing here assumes it.
pub type S = Vec<(T, V)>;

#[derive(Clone, Copy, Debug)]
pub enum F1 {
    Add(i64),
    Mul(i64),
}

#[derive(Clone, Copy, Debug)]
pub enum P {
    Even,
    Gt(i64),
}

#[derive(Clone, Copy, Debug)]
pub enum F2 {
    Add,
    Sub,
    Left,
    Right,
}

/// What `Split` makes of one event: `One` is Sodium's `defer`, a split of
/// a singleton; `Spread(k)` makes `v*10, v*10+1, ...`, `k` children.
#[derive(Clone, Copy, Debug)]
pub enum Fan {
    One,
    Spread(u8),
}

/// `construct`'s body, the `a -> Reactive b` of `Execute (MapS f s)`: runs
/// at the event's time with the event's value.
pub type Body = Rc<dyn Fn(&T, &V) -> V>;

pub enum Kind {
    // Streams.
    Mk(S),
    Never,
    Map(F1, Rc<Node>),
    Filter(P, Rc<Node>),
    Merge(Rc<Node>, Rc<Node>, F2),
    Snapshot(F2, Rc<Node>, Rc<Node>),
    SwitchS(Rc<Node>),
    Execute(Rc<Node>, Body),
    Updates(Rc<Node>),
    Value(Rc<Node>, T),
    /// `Split`; `sorted` is patch F7, a stable sort by time.
    Split(Rc<Node>, Fan, bool),
    CutS(Rc<Node>, T),
    // Cells.
    Concrete(V, S),
    Constant(V),
    Hold(V, Rc<Node>, T),
    MapC(F1, Rc<Node>),
    /// `MapC (\i -> choices !! (i mod n))`: a cell of cells or of streams.
    Pick(Rc<Node>, Vec<V>),
    /// `Apply (MapC f a) b`, which is `lift2`.
    Apply2(F2, Rc<Node>, Rc<Node>),
    SwitchC(Rc<Node>, T),
    CutC(Rc<Node>, T),
}

pub struct Node {
    pub kind: Kind,
    den: OnceCell<Den>,
}

pub enum Den {
    S(S),
    C(V, S),
}

impl fmt::Debug for V {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            V::I(i) => write!(f, "{i}"),
            V::N(n) if n.is_cell() => write!(f, "<cell>"),
            V::N(_) => write!(f, "<stream>"),
        }
    }
}

impl F1 {
    fn ap(self, a: i64) -> i64 {
        match self {
            F1::Add(k) => a.wrapping_add(k),
            F1::Mul(k) => a.wrapping_mul(k),
        }
    }
}

impl P {
    fn ap(self, a: i64) -> bool {
        match self {
            P::Even => a % 2 == 0,
            P::Gt(k) => a > k,
        }
    }
}

impl F2 {
    fn ap(self, a: i64, b: i64) -> i64 {
        match self {
            F2::Add => a.wrapping_add(b),
            F2::Sub => a.wrapping_sub(b),
            F2::Left => a,
            F2::Right => b,
        }
    }
}

impl Fan {
    pub fn len(self) -> usize {
        match self {
            Fan::One => 1,
            Fan::Spread(k) => k as usize,
        }
    }

    fn ap(self, v: i64) -> Vec<i64> {
        match self {
            Fan::One => vec![v],
            Fan::Spread(k) => (0..k as i64)
                .map(|i| v.wrapping_mul(10).wrapping_add(i))
                .collect(),
        }
    }
}

pub fn int(v: &V) -> i64 {
    match v {
        V::I(i) => *i,
        V::N(_) => panic!("expected an Int"),
    }
}

pub fn node(v: &V) -> &Rc<Node> {
    match v {
        V::N(n) => n,
        V::I(_) => panic!("expected a node"),
    }
}

impl Node {
    pub fn new(kind: Kind) -> Rc<Node> {
        Rc::new(Node {
            kind,
            den: OnceCell::new(),
        })
    }

    pub fn is_cell(&self) -> bool {
        matches!(
            self.kind,
            Kind::Concrete(..)
                | Kind::Constant(_)
                | Kind::Hold(..)
                | Kind::MapC(..)
                | Kind::Pick(..)
                | Kind::Apply2(..)
                | Kind::SwitchC(..)
                | Kind::CutC(..)
        )
    }

    pub fn den(&self) -> &Den {
        self.den.get_or_init(|| eval(&self.kind))
    }

    /// `occs`.
    pub fn occs(&self) -> &S {
        match self.den() {
            Den::S(s) => s,
            Den::C(..) => panic!("occs of a cell"),
        }
    }

    /// `steps`.
    pub fn steps(&self) -> (&V, &S) {
        match self.den() {
            Den::C(i, s) => (i, s),
            Den::S(_) => panic!("steps of a stream"),
        }
    }
}

/// `at`: the last value strictly before t. Filters the whole list, as the
/// text does, so it reads an out-of-order list the way the text would.
pub fn at(init: &V, sts: &S, t: &T) -> V {
    sts.iter()
        .rev()
        .find(|(tt, _)| tt < t)
        .map(|(_, a)| a)
        .unwrap_or(init)
        .clone()
}

/// `chopFront`.
pub fn chop_front(init: &V, sts: &S, t0: &T) -> (V, S) {
    (at(init, sts, t0), from(sts, t0))
}

/// The events at or after t0.
pub fn from(s: &S, t0: &T) -> S {
    s.iter().filter(|(t, _)| t >= t0).cloned().collect()
}

/// `coalesce f`: adjacent events at one time fold left.
fn coalesce(s: S, f: impl Fn(&V, &V) -> V) -> S {
    let mut out: S = Vec::with_capacity(s.len());
    for (t, a) in s {
        match out.last_mut() {
            Some((lt, la)) if *lt == t => *la = f(la, &a),
            _ => out.push((t, a)),
        }
    }
    out
}

/// `coalesce (flip const)`: the last of each run wins.
fn coalesce_last(s: S) -> S {
    coalesce(s, |_, b| b.clone())
}

fn eval(k: &Kind) -> Den {
    match k {
        Kind::Mk(s) => Den::S(s.clone()),
        Kind::Never => Den::S(vec![]),
        Kind::Map(f, s) => Den::S(
            s.occs()
                .iter()
                .map(|(t, a)| (t.clone(), V::I(f.ap(int(a)))))
                .collect(),
        ),
        Kind::Filter(p, s) => Den::S(
            s.occs()
                .iter()
                .filter(|(_, a)| p.ap(int(a)))
                .cloned()
                .collect(),
        ),
        Kind::Merge(a, b, f) => {
            // knit, exactly as the text: ties go left, and the rest appended.
            let (xs, ys) = (a.occs(), b.occs());
            let (mut i, mut j) = (0, 0);
            let mut out = Vec::new();
            while i < xs.len() && j < ys.len() {
                if xs[i].0 <= ys[j].0 {
                    out.push(xs[i].clone());
                    i += 1;
                } else {
                    out.push(ys[j].clone());
                    j += 1;
                }
            }
            out.extend_from_slice(&xs[i..]);
            out.extend_from_slice(&ys[j..]);
            Den::S(coalesce(out, |x, y| V::I(f.ap(int(x), int(y)))))
        }
        Kind::Snapshot(f, s, c) => {
            let (ci, cs) = c.steps();
            Den::S(
                s.occs()
                    .iter()
                    .map(|(t, a)| (t.clone(), V::I(f.ap(int(a), int(&at(ci, cs, t))))))
                    .collect(),
            )
        }
        Kind::SwitchS(c) => {
            // scan: the inner selected by a step at t1 gives the events in
            // (t1, t2]; the first inner, everything up to the first step.
            let (a, sts) = c.steps();
            let mut out = Vec::new();
            let mut lo: Option<&T> = None;
            let mut cur = a;
            for (t1, a1) in sts {
                out.extend(
                    node(cur)
                        .occs()
                        .iter()
                        .filter(|(t, _)| lo.is_none_or(|l| t > l) && t <= t1)
                        .cloned(),
                );
                lo = Some(t1);
                cur = a1;
            }
            out.extend(
                node(cur)
                    .occs()
                    .iter()
                    .filter(|(t, _)| lo.is_none_or(|l| t > l))
                    .cloned(),
            );
            Den::S(out)
        }
        Kind::Execute(s, body) => Den::S(
            s.occs()
                .iter()
                .map(|(t, a)| (t.clone(), body(t, a)))
                .collect(),
        ),
        Kind::Updates(c) => Den::S(c.steps().1.clone()),
        Kind::Value(c, t0) => {
            let (i, s) = c.steps();
            let (a, sts) = chop_front(i, s, t0);
            let mut out = vec![(t0.clone(), a)];
            out.extend(sts);
            Den::S(coalesce_last(out))
        }
        Kind::Split(s, fan, sorted) => {
            // concatMap split (coalesce (++) (occs s)), lists as Fan images.
            let mut lists: Vec<(T, Vec<i64>)> = Vec::new();
            for (t, a) in s.occs() {
                let parts = fan.ap(int(a));
                match lists.last_mut() {
                    Some((lt, l)) if lt == t => l.extend(parts),
                    _ => lists.push((t.clone(), parts)),
                }
            }
            let mut out: S = Vec::new();
            for (t, l) in lists {
                for (n, v) in l.into_iter().enumerate() {
                    let mut c = t.clone();
                    c.push(n as i32);
                    out.push((c, V::I(v)));
                }
            }
            if *sorted {
                out.sort_by(|x, y| x.0.cmp(&y.0));
            }
            Den::S(out)
        }
        Kind::CutS(s, t0) => Den::S(from(s.occs(), t0)),
        Kind::Concrete(i, s) => Den::C(i.clone(), s.clone()),
        Kind::Constant(v) => Den::C(v.clone(), vec![]),
        Kind::Hold(a, s, t0) => Den::C(a.clone(), coalesce_last(from(s.occs(), t0))),
        Kind::MapC(f, c) => {
            let (i, s) = c.steps();
            Den::C(
                V::I(f.ap(int(i))),
                s.iter()
                    .map(|(t, a)| (t.clone(), V::I(f.ap(int(a)))))
                    .collect(),
            )
        }
        Kind::Pick(c, choices) => {
            let pick = |v: &V| choices[int(v).rem_euclid(choices.len() as i64) as usize].clone();
            let (i, s) = c.steps();
            Den::C(
                pick(i),
                s.iter().map(|(t, a)| (t.clone(), pick(a))).collect(),
            )
        }
        Kind::Apply2(f, a, b) => {
            // Apply's knit over (MapC f a) and b.
            let (ai, as_) = a.steps();
            let (bi, bs) = b.steps();
            let (mut x, mut y) = (int(ai), int(bi));
            let (mut i, mut j) = (0, 0);
            let mut out = Vec::new();
            loop {
                let t = match (as_.get(i), bs.get(j)) {
                    (None, None) => break,
                    (Some((ta, va)), Some((tb, _))) if ta < tb => {
                        x = int(va);
                        i += 1;
                        ta
                    }
                    (Some((ta, _)), Some((tb, vb))) if ta > tb => {
                        y = int(vb);
                        j += 1;
                        tb
                    }
                    (Some((ta, va)), Some((_, vb))) => {
                        x = int(va);
                        y = int(vb);
                        i += 1;
                        j += 1;
                        ta
                    }
                    (Some((ta, va)), None) => {
                        x = int(va);
                        i += 1;
                        ta
                    }
                    (None, Some((tb, vb))) => {
                        y = int(vb);
                        j += 1;
                        tb
                    }
                };
                out.push((t.clone(), V::I(f.ap(x, y))));
            }
            Den::C(V::I(f.ap(int(ai), int(bi))), out)
        }
        Kind::SwitchC(c, t0) => switch_c(c, t0),
        Kind::CutC(c, t0) => {
            let (i, s) = c.steps();
            let (a, sts) = chop_front(i, s, t0);
            Den::C(a, sts)
        }
    }
}

/// `steps (SwitchC c t0)`, as the text writes it. `normalize` is bound to
/// the switch's own t0, not to the scan's, as in the text's `where`.
fn switch_c(c: &Rc<Node>, t0: &T) -> Den {
    let (a, sts) = c.steps();
    let at_c = at(a, sts, t0);
    let (ii, is) = node(&at_c).steps();
    let init = at(ii, is, t0);

    let normalize = |(b, s): (V, S)| -> (V, S) {
        match s.first() {
            Some((t1, a)) if t1 == t0 => (a.clone(), s[1..].to_vec()),
            _ => (b, s),
        }
    };
    let mut out: S = Vec::new();
    let mut lo = t0.clone();
    let mut cur = a;
    for (t1, a1) in sts {
        let (ci, cs) = node(cur).steps();
        let (b, s) = chop_front(ci, cs, &lo);
        let s: S = s.into_iter().filter(|(t, _)| t < t1).collect();
        let (b, s) = normalize((b, s));
        out.push((lo.clone(), b));
        out.extend(s);
        lo = t1.clone();
        cur = a1;
    }
    let (ci, cs) = node(cur).steps();
    let (b, s) = normalize(chop_front(ci, cs, &lo));
    out.push((lo, b));
    out.extend(s);
    Den::C(init, coalesce_last(out))
}

/// Two values the same as seen from t0 on: equal Ints, or nodes whose
/// denotations agree from t0 on.
pub fn eq_from(a: &V, b: &V, t0: &T) -> bool {
    match (a, b) {
        (V::I(x), V::I(y)) => x == y,
        (V::N(x), V::N(y)) => {
            Rc::ptr_eq(x, y)
                || match (x.den(), y.den()) {
                    (Den::S(xs), Den::S(ys)) => eq_s_from(xs, ys, t0),
                    (Den::C(xi, xs), Den::C(yi, ys)) => {
                        eq_from(&at(xi, xs, t0), &at(yi, ys, t0), t0) && eq_s_from(xs, ys, t0)
                    }
                    _ => false,
                }
        }
        _ => false,
    }
}

/// Two event or step lists the same from t0 on: the same times, in the same
/// order, with values the same from when they are delivered. A cell or
/// stream that arrives at t is only observable from t on (FRPNow's relation
/// on nested behaviours), and one built at t has no history before it.
pub fn eq_s_from(xs: &S, ys: &S, t0: &T) -> bool {
    let xs = xs.iter().filter(|(t, _)| t >= t0);
    let mut ys = ys.iter().filter(|(t, _)| t >= t0);
    for (tx, vx) in xs {
        match ys.next() {
            Some((ty, vy)) if tx == ty && eq_from(vx, vy, tx) => {}
            _ => return false,
        }
    }
    ys.next().is_none()
}

/// Whether the node's times strictly increase, as `S a` and `C a` promise.
pub fn in_order(n: &Node) -> bool {
    let s = match n.den() {
        Den::S(s) => s,
        Den::C(_, s) => s,
    };
    s.windows(2).all(|w| w[0].0 < w[1].0)
}

pub fn show_t(t: &T) -> String {
    format!(
        "[{}]",
        t.iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(",")
    )
}

pub fn show_s(s: &S) -> String {
    s.iter()
        .map(|(t, v)| format!("{}:{v:?}", show_t(t)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A node's denotation, as the draft issues print it.
pub fn show(n: &Node) -> String {
    match n.den() {
        Den::S(s) => format!("[{}]", show_s(s)),
        Den::C(i, s) => format!("({i:?}, [{}])", show_s(s)),
    }
}
