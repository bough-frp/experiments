//! Does a topological order that moves only the small side of a reorder
//! bound a switch move's same-instant cycle check below the unbounded
//! upstream walk on every workload, including those that build their inners
//! during the instant?
//!
//! `rfd-0005-bounded-relink-check` found Pearce and Kelly's `u32` array 40
//! times cheaper than the walk when inners are prebuilt, and 20 to 230 times
//! dearer when they are built during the instant. A node built appends at
//! the end of the order, so linking a new events screen into its
//! `switch_stream` invalidates the order, and PK's forward search from the
//! switch covers the switch's whole downstream: the state, and everything
//! that reads it. The side that actually has to move is the new screen,
//! about a dozen nodes, and it belongs just before the switch. An array
//! can't put it there without shifting everything in between; an
//! order-maintenance list can.
//!
//! Both variants here keep the order as an order-maintenance list: a doubly
//! linked list of nodes with `u64` labels that compare in O(1), relabelled
//! when a gap runs out after Bender, Cole, Demaine, Farach-Colton and Zito's
//! simplified structure (the kind Acar's thesis uses, per synthesis 08), so
//! any set of nodes can be moved just after or just before any node. A link
//! `x → y` with `x` already before `y` costs one comparison, as in PK.
//! Otherwise:
//!
//! - [`Back`]: search backward from the new inner `x`, over dependencies,
//!   only among nodes after the switch `y`. Reaching `y` is a cycle, since
//!   every path from `y` to `x` lies between them in the order. Otherwise
//!   move what it found just before `y`, in its old relative order. This is
//!   "place the new nodes just before the switch", done at link time,
//!   because a node's switch isn't known when it is built.
//! - [`TwoWay`]: search forward from `y` and backward from `x` at once, one
//!   node from each side in turn, until the two meet (a cycle) or one side
//!   runs out. Move that side: the forward set just after `x`, or the
//!   backward set just before `y`. This is Haeupler, Kavitha, Mathew, Sen
//!   and Tarjan's two-way search in its simplest form, "stop at the first
//!   side to finish", not their compatible search with its O(m^3/2) bound;
//!   it visits at most about twice the smaller side.
//!
//! Both do what PK does around the search: a transaction's unlinks first,
//! so F46's reversal is never refused, and on a refusal every old inner
//! relinked through the order. The graph, the workloads, the walk and PK
//! are the earlier probe's, called through its module.

use crate::rfd_0005_bounded_relink_check::{Checker, Counts, Graph, Id, Kind, Move};

/// No node: the end of the list either way.
const NONE: Id = Id::MAX;

/// Labels live in `1..LIMIT`; 0 stands for "before the head".
const LIMIT: u64 = 1 << 62;

/// The spacing of a fresh label: at the start, and after the tail.
const GAP: u64 = 1 << 32;

/// An order-maintenance list over node ids.
#[derive(Clone, Default)]
pub struct Order {
    label: Vec<u64>,
    prev: Vec<Id>,
    next: Vec<Id>,
    head: Id,
    tail: Id,
    /// Nodes that already held a label and were given another by a
    /// relabelling: the list's upkeep, apart from the nodes moved.
    pub relabeled: u64,
    /// Scratch for a relabelling.
    run: Vec<Id>,
}

impl Order {
    /// The list in `order`, evenly spaced.
    fn new(order: &[Id], len: usize) -> Self {
        let mut me = Order {
            label: vec![0; len],
            prev: vec![NONE; len],
            next: vec![NONE; len],
            head: NONE,
            tail: NONE,
            ..Order::default()
        };
        for &n in order {
            me.push(n);
        }
        me
    }

    pub fn label(&self, n: Id) -> u64 {
        self.label[n as usize]
    }

    /// Makes room for nodes up to `len`, before they are appended.
    fn grow(&mut self, len: usize) {
        self.label.resize(len, 0);
        self.prev.resize(len, NONE);
        self.next.resize(len, NONE);
    }

    /// Appends a node not yet in the list, which has room for it.
    fn push(&mut self, n: Id) {
        if self.tail == NONE || self.label(self.tail) >= LIMIT - GAP {
            let tail = (self.tail != NONE).then_some(self.tail);
            self.insert_after(tail, &[n]);
            return;
        }
        // The usual case, a fresh label after the tail's.
        self.label[n as usize] = self.label(self.tail) + GAP;
        self.prev[n as usize] = self.tail;
        self.next[n as usize] = NONE;
        self.next[self.tail as usize] = n;
        self.tail = n;
    }

    /// Takes `n` out of the list; its label is stale until it is inserted.
    fn remove(&mut self, n: Id) {
        let (p, q) = (self.prev[n as usize], self.next[n as usize]);
        if p == NONE {
            self.head = q;
        } else {
            self.next[p as usize] = q;
        }
        if q == NONE {
            self.tail = p;
        } else {
            self.prev[q as usize] = p;
        }
    }

    /// Links `nodes`, in order, just after `a` (or at the head), and labels
    /// them, relabelling a range around `a` if the gap is too small.
    fn insert_after(&mut self, a: Option<Id>, nodes: &[Id]) {
        let lo = a.map_or(0, |a| self.label(a));
        let after = a.map_or(self.head, |a| self.next[a as usize]);
        let hi = if after == NONE {
            LIMIT
        } else {
            self.label(after)
        };
        let mut p = a.unwrap_or(NONE);
        for &n in nodes {
            self.prev[n as usize] = p;
            if p == NONE {
                self.head = n;
            } else {
                self.next[p as usize] = n;
            }
            p = n;
        }
        self.next[p as usize] = after;
        if after == NONE {
            self.tail = p;
        } else {
            self.prev[after as usize] = p;
        }
        let k = nodes.len() as u64;
        if hi - lo > k {
            // Spaced no wider than a fresh label, so appends at the tail
            // don't halve the space left each time.
            let step = ((hi - lo) / (k + 1)).min(GAP);
            for (j, &n) in nodes.iter().enumerate() {
                self.label[n as usize] = lo + step * (j as u64 + 1);
            }
        } else {
            self.relabel(a, nodes);
        }
    }

    /// The gap after `a` is too small for `nodes`, which are already linked
    /// after it. Finds the smallest aligned label range around `a` that
    /// doesn't overflow with them counted in, and spreads everything in it
    /// evenly over it.
    fn relabel(&mut self, a: Option<Id>, nodes: &[Id]) {
        let base = a.map_or(0, |a| self.label(a));
        let k = nodes.len() as u64;
        // The labelled nodes inside the range run from `first` (or the
        // head) to `last`, `count` of them, with the new nodes among them.
        let mut first = a.unwrap_or(NONE);
        let mut last = nodes[nodes.len() - 1];
        let mut count = a.is_some() as u64;
        // A range of width 2^i overflows when it would hold more than
        // (2 / T)^i nodes, with Bender et al.'s density parameter T = 1.5.
        let mut capacity = 1.0f64;
        for i in 1..=62 {
            capacity *= 4.0 / 3.0;
            // Aligned, so `start + 2^i` never passes `LIMIT`.
            let start = base & !((1u64 << i) - 1);
            let end = start + (1u64 << i);
            if first != NONE {
                let mut b = self.prev[first as usize];
                while b != NONE && self.label(b) >= start {
                    first = b;
                    count += 1;
                    b = self.prev[b as usize];
                }
            }
            let mut c = self.next[last as usize];
            while c != NONE && self.label(c) < end {
                last = c;
                count += 1;
                c = self.next[c as usize];
            }
            if ((count + k) as f64) < capacity || i == 62 {
                self.run.clear();
                let mut n = if first == NONE { self.head } else { first };
                loop {
                    self.run.push(n);
                    if n == last {
                        break;
                    }
                    n = self.next[n as usize];
                }
                // Labels in `start.max(1)..end`, evenly.
                let lo = start.max(1) - 1;
                let step = (end - lo) / (self.run.len() as u64 + 1);
                assert!(step > 0, "the order-maintenance list is full");
                for (j, &n) in self.run.iter().enumerate() {
                    self.label[n as usize] = lo + step * (j as u64 + 1);
                }
                self.relabeled += count;
                return;
            }
        }
        unreachable!("level 62 always relabels");
    }

    /// Moves `side`, sorted by label, just after `anchor`, which isn't in it.
    fn move_after(&mut self, anchor: Id, side: &[Id]) {
        for &n in side {
            self.remove(n);
        }
        self.insert_after(Some(anchor), side);
    }

    /// Moves `side`, sorted by label, just before `anchor`, which isn't in
    /// it.
    fn move_before(&mut self, anchor: Id, side: &[Id]) {
        for &n in side {
            self.remove(n);
        }
        let p = self.prev[anchor as usize];
        self.insert_after((p != NONE).then_some(p), side);
    }

    /// Whether the list holds `len` nodes, linked both ways, in increasing
    /// label order.
    pub fn is_consistent(&self, len: usize) -> bool {
        let (mut n, mut p) = (self.head, NONE);
        let mut last = 0;
        let mut seen = 0;
        while n != NONE {
            if self.label(n) <= last || self.label(n) >= LIMIT || self.prev[n as usize] != p {
                return false;
            }
            last = self.label(n);
            seen += 1;
            (p, n) = (n, self.next[n as usize]);
        }
        seen == len && self.tail == p
    }
}

/// A reusable search: a stack, what it has expanded, and a per-node stamp.
#[derive(Clone, Default)]
struct Search {
    stack: Vec<Id>,
    found: Vec<Id>,
    stamp: Vec<u32>,
    epoch: u32,
}

impl Search {
    fn begin(&mut self, len: usize, from: Id) {
        self.stamp.resize(len, 0);
        self.epoch += 1;
        self.stack.clear();
        self.found.clear();
        self.stack.push(from);
        self.mark(from);
    }

    fn mark(&mut self, n: Id) {
        self.stamp[n as usize] = self.epoch;
    }

    fn seen(&self, n: Id) -> bool {
        self.stamp[n as usize] == self.epoch
    }
}

/// What one step of a search met.
enum Step {
    More,
    Done,
    Cycle,
}

/// How an invalidating link's search ended.
enum Outcome {
    /// The backward search finished first; its side moves before `y`.
    Back,
    /// The forward search did; its side moves after `x`.
    Forward,
    Cycle,
}

/// The order-maintenance checker; `TWO_WAY` picks the search.
#[derive(Clone, Default)]
pub struct Om<const TWO_WAY: bool> {
    pub order: Order,
    forward: Search,
    backward: Search,
    counts: Counts,
}

/// Backward from the new inner only; what it finds goes before the switch.
pub type Back = Om<false>;

/// Both ways at once; the side that finishes first moves.
pub type TwoWay = Om<true>;

impl<const TWO_WAY: bool> Om<TWO_WAY> {
    /// Expands one node of the forward search from `y`, among nodes before
    /// `x`. Reaching `x`, or a node the backward search found, is a cycle.
    fn forward_step(&mut self, g: &Graph, x: Id) -> Step {
        let Some(n) = self.forward.stack.pop() else {
            return Step::Done;
        };
        self.forward.found.push(n);
        let ub = self.order.label(x);
        for &w in &g.dependents[n as usize] {
            if w == x || self.backward.seen(w) {
                return Step::Cycle;
            }
            if self.order.label(w) < ub && !self.forward.seen(w) {
                self.forward.mark(w);
                self.forward.stack.push(w);
            }
        }
        Step::More
    }

    /// Expands one node of the backward search from `x`, among nodes after
    /// `y`. Reaching `y`, or (two-way) a node the forward search found, is
    /// a cycle.
    fn backward_step(&mut self, g: &Graph, y: Id) -> Step {
        let Some(n) = self.backward.stack.pop() else {
            return Step::Done;
        };
        self.backward.found.push(n);
        let lb = self.order.label(y);
        for &w in &g.deps[n as usize] {
            if w == y || (TWO_WAY && self.forward.seen(w)) {
                return Step::Cycle;
            }
            if self.order.label(w) > lb && !self.backward.seen(w) {
                self.backward.mark(w);
                self.backward.stack.push(w);
            }
        }
        Step::More
    }

    /// Links `x → y` unless it closes a cycle, keeping the order.
    fn insert(&mut self, g: &mut Graph, x: Id, y: Id, kind: Kind) -> bool {
        if self.order.label(x) < self.order.label(y) {
            g.link(x, y);
            return true;
        }
        self.counts.invalid[kind as usize] += 1;
        self.backward.begin(g.len(), x);
        if TWO_WAY {
            self.forward.begin(g.len(), y);
        }
        let outcome = loop {
            match self.backward_step(g, y) {
                Step::Done => break Outcome::Back,
                Step::Cycle => break Outcome::Cycle,
                Step::More => {}
            }
            if TWO_WAY {
                match self.forward_step(g, x) {
                    Step::Done => break Outcome::Forward,
                    Step::Cycle => break Outcome::Cycle,
                    Step::More => {}
                }
            }
        };
        let forward = if TWO_WAY { self.forward.found.len() } else { 0 };
        self.counts.visited[kind as usize] += (self.backward.found.len() + forward) as u64;
        let order = &mut self.order;
        match outcome {
            Outcome::Cycle => return false,
            Outcome::Back => {
                let side = &mut self.backward.found;
                side.sort_unstable_by_key(|&n| order.label(n));
                order.move_before(y, side);
            }
            Outcome::Forward => {
                let side = &mut self.forward.found;
                side.sort_unstable_by_key(|&n| order.label(n));
                order.move_after(x, side);
            }
        }
        g.link(x, y);
        true
    }
}

impl<const TWO_WAY: bool> Checker for Om<TWO_WAY> {
    fn new(g: &Graph) -> Self {
        // Kahn's algorithm over the whole graph.
        let mut indeg: Vec<u32> = g.deps.iter().map(|d| d.len() as u32).collect();
        let mut order: Vec<Id> = (0..g.len() as Id)
            .filter(|&n| indeg[n as usize] == 0)
            .collect();
        let mut k = 0;
        while k < order.len() {
            let n = order[k];
            k += 1;
            for &d in &g.dependents[n as usize] {
                indeg[d as usize] -= 1;
                if indeg[d as usize] == 0 {
                    order.push(d);
                }
            }
        }
        assert_eq!(order.len(), g.len(), "the build is acyclic");
        Om {
            order: Order::new(&order, g.len()),
            ..Om::default()
        }
    }

    /// A node built depends only on older ones, so it goes at the end.
    fn built(&mut self, g: &Graph, first: Id) {
        self.order.grow(g.len());
        for n in first..g.len() as Id {
            self.order.push(n);
        }
    }

    fn commit(&mut self, g: &mut Graph, moves: &[Move]) -> bool {
        for m in moves {
            self.counts.moves[m.kind as usize] += 1;
        }
        // Deletions first: they never invalidate the order, and after them
        // each graph on the way is a subgraph of the final one (F46).
        for m in moves {
            g.unlink(m.from, m.switch);
        }
        let mut linked = 0;
        while linked < moves.len() {
            let m = moves[linked];
            if !self.insert(g, m.to, m.switch, m.kind) {
                break;
            }
            linked += 1;
        }
        if linked == moves.len() {
            return true;
        }
        // Refused: undo what was linked and relink the old inners through
        // the order, since a reorder may have put one after its switch. The
        // old graph was acyclic, so this cannot fail.
        self.counts.refused += 1;
        for m in &moves[..linked] {
            g.unlink(m.to, m.switch);
        }
        for m in moves {
            let relinked = self.insert(g, m.from, m.switch, m.kind);
            debug_assert!(relinked);
        }
        false
    }

    fn counts(&self) -> &Counts {
        &self.counts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rfd_0005_bounded_relink_check::{
        Baseline, KINDS, Pk, Run, TXS, Workload, generate, workloads,
    };

    /// Every edge goes forward in the order, and the list is sound.
    fn is_topological<const T: bool>(g: &Graph, c: &Om<T>) -> bool {
        c.order.is_consistent(g.len())
            && (0..g.len()).all(|n| {
                g.deps[n]
                    .iter()
                    .all(|&d| c.order.label(d) < c.order.label(n as Id))
            })
    }

    /// F46, in the workload's own gadget: B from p to c and A from x to y
    /// together is legal; B back to p alone then closes a cycle; both back
    /// together is legal again.
    fn f46_accepted<const T: bool>() {
        let w = generate(&workloads(4)[0].1);
        // Transaction 2 is F46's reversal, and 3 undoes it.
        let (there, back) = (&w.txs[2].moves, &w.txs[3].moves);
        assert!(there.iter().chain(back).all(|m| m.kind == Kind::F46));
        let mut run = Run::<Om<T>>::new(&w);
        assert!(run.checker.commit(&mut run.graph, there), "F46 refused");
        assert!(is_topological(&run.graph, &run.checker));
        let b = there[0];
        let b_back = [Move {
            from: b.to,
            to: b.from,
            ..b
        }];
        assert!(
            !run.checker.commit(&mut run.graph, &b_back),
            "a cycle accepted"
        );
        assert!(is_topological(&run.graph, &run.checker));
        assert!(
            run.checker.commit(&mut run.graph, back),
            "F46 undone refused"
        );
        assert!(is_topological(&run.graph, &run.checker));
    }

    #[test]
    fn f46_is_accepted_by_every_variant() {
        f46_accepted::<false>();
        f46_accepted::<true>();
    }

    /// Runs `w` through `C`, asserting `check` after every transaction and
    /// the walk's verdicts at the end.
    fn agrees<C: Checker>(w: &Workload, check: impl Fn(&Graph, &C) -> bool) -> Run<C> {
        let mut run = Run::<C>::new(w);
        let mut refused = Vec::new();
        for t in &w.txs {
            refused.push(!run.tx(t));
            assert!(check(&run.graph, &run.checker), "the order broke");
        }
        assert_eq!(refused, w.refused, "verdicts differ from the walk's");
        run
    }

    /// Every variant refuses exactly the transactions the walk refuses, on
    /// every workload, also with many more cyclic moves, and keeps a
    /// topological order throughout.
    #[test]
    fn every_variant_refuses_what_the_walk_refuses() {
        for (_, mut p) in workloads(120) {
            for bad in [p.p_bad, 0.3] {
                p.p_bad = bad;
                let w = generate(&p);
                assert!(bad < 0.3 || w.refused.iter().any(|&r| r));
                agrees::<Back>(&w, is_topological);
                agrees::<TwoWay>(&w, is_topological);
            }
        }
    }

    /// Relabelling keeps the list in order when a gap runs out.
    #[test]
    fn relabelling_keeps_the_order() {
        let mut o = Order::new(&[0, 1], 2);
        // Each node goes just before node 1, which halves the gap there
        // until it is gone.
        for n in 2..3002 {
            o.grow(n as usize + 1);
            o.push(n);
            o.move_before(1, &[n]);
            assert!(o.is_consistent(n as usize + 1));
        }
        assert!(o.relabeled > 0);
        let mut seq = Vec::new();
        let mut n = o.head;
        while n != NONE {
            seq.push(n);
            n = o.next[n as usize];
        }
        let mut want = vec![0];
        want.extend(2..3002);
        want.push(1);
        assert_eq!(seq, want);
    }

    /// The counts the note quotes: per workload and kind of move, nodes
    /// visited by the walk, PK and each variant, and how often a link
    /// invalidates each order. Run with `--nocapture`.
    #[test]
    fn counts() {
        println!("{TXS} transactions per workload, 2 slots each");
        for (name, p) in workloads(TXS) {
            let w = generate(&p);
            let base = agrees::<Baseline>(&w, |_, _| true);
            let pk = agrees::<Pk>(&w, |_, _| true);
            let back = agrees::<Back>(&w, |_, _| true);
            let two = agrees::<TwoWay>(&w, |_, _| true);
            let (base, pk) = (base.checker.counts(), pk.checker.counts());
            let (bc, tc) = (back.checker.counts(), two.checker.counts());
            println!();
            println!(
                "{name} (p_new {}, p_rev {}): {} nodes, {} built during the transactions, \
                 {} moves, {} transactions refused",
                p.p_new,
                p.p_rev,
                w.graph.len(),
                w.built(),
                w.moves(),
                base.refused,
            );
            println!(
                "  {:<6} {:>6} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}",
                "kind", "moves", "walk", "pk", "back", "twoway", "pk inv", "bk inv", "tw inv"
            );
            let row = |label: &str, r: [u64; 8]| {
                let n = r[0] as f64;
                println!(
                    "  {label:<6} {:>6} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>7.1}% {:>7.1}% {:>7.1}%",
                    r[0],
                    r[1] as f64 / n,
                    r[2] as f64 / n,
                    r[3] as f64 / n,
                    r[4] as f64 / n,
                    100.0 * r[5] as f64 / n,
                    100.0 * r[6] as f64 / n,
                    100.0 * r[7] as f64 / n,
                );
            };
            let mut total = [0u64; 8];
            for k in KINDS {
                let i = k as usize;
                if base.moves[i] == 0 {
                    continue;
                }
                let r = [
                    base.moves[i],
                    base.visited[i],
                    pk.visited[i],
                    bc.visited[i],
                    tc.visited[i],
                    pk.invalid[i],
                    bc.invalid[i],
                    tc.invalid[i],
                ];
                row(&format!("{k:?}").to_lowercase(), r);
                for (t, x) in total.iter_mut().zip(r) {
                    *t += x;
                }
            }
            row("all", total);
            let per = |r: u64| r as f64 / total[0] as f64;
            let (br, tr) = (back.checker.order.relabeled, two.checker.order.relabeled);
            println!(
                "  relabelled by the list: back {br} nodes ({:.1} a move), twoway {tr} ({:.1} a move)",
                per(br),
                per(tr),
            );
        }
        println!();
        println!("nodes visited per move: walked (walk), searched forward and back (pk,");
        println!("twoway), or back only (back); inv is the share of moves whose new link");
        println!("was against that order, so reordered it or found a cycle. A refused");
        println!("transaction's relinks are counted under their moves' kinds. Relabelled");
        println!("counts nodes the list gave new labels to make room, apart from those moved.");
    }
}
