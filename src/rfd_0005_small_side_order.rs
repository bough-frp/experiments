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
//!
//! On that UI-shaped graph neither search covers much, which isn't a worst
//! case. [`Adversary`] asks where they stop being cheap: a new inner whose
//! upstream all sits after the switch in the order, and a switch whose
//! downstream all sits before the new inner, each side sized apart, so that
//! `back` has to cover the first and PK both, while two-way should cover
//! about twice the smaller; and a cyclic form, where both sides lie on the
//! cycle.
//!
//! Two follow-ups. [`Adversary::mixed`] gives the new inner a large old
//! upstream as well, built before the switch so it sits before it in the
//! order, where the UI graph gave the list its advantage: the walk covers
//! it and `back` doesn't, so sweeping it against the new side finds where
//! the two cost the same. And the moved set always goes into the same gap,
//! just before the switch, which the even spacing fills at once, so the
//! next move relabels. [`BackFresh`] and [`TwoWayFresh`] place a moved set
//! in the half of its gap away from where the next one will go, and when
//! the gap is too small, relabel a neighbourhood once so that half of it is
//! left open there: see [`Order::insert_after`].
//!
//! A third. With fresh spacing, `back` on the adversary spends most of its
//! time sorting what it found by label before moving it, to keep its old
//! relative order. Any topological order of the set will do, though: a
//! node the set depends on outside it sits before the switch, and one that
//! depends on it after the switch, either way. `back`'s search records a
//! node when it pops it, before its dependencies, so neither that order nor
//! its reverse is topological in general. [`BackNoSort`] searches depth
//! first instead and records a node when it finishes, after every
//! dependency it reaches in the set, and moves the set in that post-order,
//! unsorted.

use crate::rfd_0005_bounded_relink_check::{Checker, Counts, Graph, Id, Kind, Move, Run};

/// No node: the end of the list either way.
const NONE: Id = Id::MAX;

/// Labels live in `1..LIMIT`; 0 stands for "before the head".
const LIMIT: u64 = 1 << 62;

/// The spacing of a fresh label: at the start, and after the tail.
const GAP: u64 = 1 << 32;

/// Which side of a moved set the next set moved to the same spot will go:
/// between it and the node after (`After`, a move before a switch) or
/// between the node before and it (`Before`, a move after a new inner).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Room {
    Before,
    After,
}

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
    /// Places a moved set in half its gap, leaving the other half open
    /// where the next set to that spot goes, instead of spreading it over
    /// the whole gap.
    fresh: bool,
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
            self.insert_after(tail, &[n], Room::After);
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
    ///
    /// Evenly, the set is spread over the whole gap, so a second set moved
    /// to the same spot finds a gap `k + 1` times smaller, and a third
    /// usually none. Fresh, the set takes the half of the gap away from
    /// `room`, where the next set will go, so the gap there only halves; and
    /// a relabelling leaves half its range open at that spot.
    fn insert_after(&mut self, a: Option<Id>, nodes: &[Id], room: Room) {
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
        // After the tail, the room belongs to the nodes appended next.
        let room = if after == NONE { Room::After } else { room };
        if self.fresh {
            // At most half the gap, and no wider than a fresh label.
            let step = ((hi - lo) / (2 * (k + 1))).min(GAP);
            if step == 0 {
                self.relabel(a, nodes, room);
                return;
            }
            for (j, &n) in nodes.iter().enumerate() {
                let j = j as u64;
                self.label[n as usize] = match room {
                    Room::After => lo + step * (j + 1),
                    Room::Before => hi - step * (k - j),
                };
            }
        } else if hi - lo > k {
            // Spaced no wider than a fresh label, so appends at the tail
            // don't halve the space left each time.
            let step = ((hi - lo) / (k + 1)).min(GAP);
            for (j, &n) in nodes.iter().enumerate() {
                self.label[n as usize] = lo + step * (j as u64 + 1);
            }
        } else {
            self.relabel(a, nodes, room);
        }
    }

    /// The gap after `a` is too small for `nodes`, which are already linked
    /// after it. Finds the smallest aligned label range around `a` that
    /// doesn't overflow with them counted in, and spreads everything in it
    /// evenly over it.
    ///
    /// Fresh, the range must not overflow with the set counted twice, for
    /// the room left beside it, and everything in it is spread evenly over
    /// its two ends, leaving half of it open on the set's `room` side.
    fn relabel(&mut self, a: Option<Id>, nodes: &[Id], room: Room) {
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
            let need = if self.fresh {
                2 * (count + k)
            } else {
                count + k
            };
            if (need as f64) < capacity || i == 62 {
                self.run.clear();
                let mut n = if first == NONE { self.head } else { first };
                loop {
                    self.run.push(n);
                    if n == last {
                        break;
                    }
                    n = self.next[n as usize];
                }
                // Labels in `start.max(1)..end`.
                let lo = start.max(1) - 1;
                let len = self.run.len() as u64;
                if self.fresh {
                    // The nodes up to the set's end on the open side go at
                    // the bottom of the range, the rest at the top, all a
                    // step apart, with the rest of the range between. The
                    // run holds the set, so `at` always finds it.
                    let at = |m: Id| self.run.iter().position(|&n| n == m).unwrap() as u64;
                    let below = match room {
                        Room::After => at(nodes[nodes.len() - 1]) + 1,
                        Room::Before => at(nodes[0]),
                    };
                    let step = (end - lo) / (2 * (len + 1));
                    assert!(step > 0, "the order-maintenance list is full");
                    for (j, &n) in self.run.iter().enumerate() {
                        let j = j as u64;
                        self.label[n as usize] = if j < below {
                            lo + step * (j + 1)
                        } else {
                            end - step * (len - j)
                        };
                    }
                } else {
                    let step = (end - lo) / (len + 1);
                    assert!(step > 0, "the order-maintenance list is full");
                    for (j, &n) in self.run.iter().enumerate() {
                        self.label[n as usize] = lo + step * (j as u64 + 1);
                    }
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
        self.insert_after(Some(anchor), side, Room::Before);
    }

    /// Moves `side`, sorted by label or otherwise topological among itself,
    /// just before `anchor`, which isn't in it.
    fn move_before(&mut self, anchor: Id, side: &[Id]) {
        for &n in side {
            self.remove(n);
        }
        let p = self.prev[anchor as usize];
        self.insert_after((p != NONE).then_some(p), side, Room::After);
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
    /// The depth-first search's path: a node and how many of its
    /// dependencies it has yet to try.
    path: Vec<(Id, u32)>,
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

/// The order-maintenance checker; `TWO_WAY` picks the search, `FRESH` the
/// spacing a moved set is given, and `POST` (backward only) a depth-first
/// search whose post-order moves without a sort.
#[derive(Clone, Default)]
pub struct Om<const TWO_WAY: bool, const FRESH: bool = false, const POST: bool = false> {
    pub order: Order,
    forward: Search,
    backward: Search,
    counts: Counts,
    /// Tests only: of the sets the stack search moved before a switch, how
    /// many, and how many were topological among themselves in the order it
    /// found them, and in the reverse.
    #[cfg(test)]
    discovery: [u64; 3],
}

/// Backward from the new inner only; what it finds goes before the switch.
pub type Back = Om<false>;

/// Both ways at once; the side that finishes first moves.
pub type TwoWay = Om<true>;

/// [`Back`], leaving room where the next moved set goes.
pub type BackFresh = Om<false, true>;

/// [`TwoWay`], leaving room where the next moved set goes.
pub type TwoWayFresh = Om<true, true>;

/// [`BackFresh`], searching depth first and moving what it finds in
/// post-order, without sorting it.
pub type BackNoSort = Om<false, true, true>;

impl<const TWO_WAY: bool, const FRESH: bool, const POST: bool> Om<TWO_WAY, FRESH, POST> {
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

    /// Back's search from `x`, among nodes after `y`, depth first, recording
    /// each node in `found` when it finishes: after every dependency of it
    /// that the search reaches, so `found` is topological among itself.
    /// Reaching `y` is a cycle. Returns the outcome and the nodes entered.
    ///
    /// A node's dependencies are tried last first, the order the stack
    /// search expands them in, so the two meet a cycle along the same paths
    /// rather than one of them taking a shortcut the other doesn't.
    fn backward_post(&mut self, g: &Graph, x: Id, y: Id) -> (Outcome, usize) {
        let lb = self.order.label(y);
        let s = &mut self.backward;
        s.begin(g.len(), x);
        s.stack.clear();
        s.path.clear();
        s.path.push((x, g.deps[x as usize].len() as u32));
        let mut entered = 1;
        while let Some((n, left)) = s.path.last_mut() {
            if *left == 0 {
                s.found.push(*n);
                s.path.pop();
                continue;
            }
            *left -= 1;
            let w = g.deps[*n as usize][*left as usize];
            if w == y {
                return (Outcome::Cycle, entered);
            }
            if self.order.label(w) > lb && !s.seen(w) {
                s.mark(w);
                s.path.push((w, g.deps[w as usize].len() as u32));
                entered += 1;
            }
        }
        (Outcome::Back, entered)
    }

    /// Links `x → y` unless it closes a cycle, keeping the order.
    fn insert(&mut self, g: &mut Graph, x: Id, y: Id, kind: Kind) -> bool {
        if self.order.label(x) < self.order.label(y) {
            g.link(x, y);
            return true;
        }
        self.counts.invalid[kind as usize] += 1;
        if POST && !TWO_WAY {
            let (outcome, entered) = self.backward_post(g, x, y);
            self.counts.visited[kind as usize] += entered as u64;
            if let Outcome::Cycle = outcome {
                return false;
            }
            // Already topological among itself: no sort.
            self.order.move_before(y, &self.backward.found);
            g.link(x, y);
            return true;
        }
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
        #[cfg(test)]
        if let Outcome::Back = outcome {
            let found = &self.backward.found;
            let rev: Vec<Id> = found.iter().rev().copied().collect();
            self.discovery[0] += 1;
            self.discovery[1] += tests::topological_among(g, found) as u64;
            self.discovery[2] += tests::topological_among(g, &rev) as u64;
        }
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

impl<const TWO_WAY: bool, const FRESH: bool, const POST: bool> Checker
    for Om<TWO_WAY, FRESH, POST>
{
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
        let mut order = Order::new(&order, g.len());
        order.fresh = FRESH;
        Om {
            order,
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

/// The sizes the adversarial cases take for each side: the new inner's
/// upstream after the switch, and the switch's downstream.
pub const SIDES: [usize; 4] = [10, 100, 1_000, 10_000];

/// The mixed adversary's sizes for the upstream every new inner shares,
/// before the switch in the order.
pub const SHARED: [usize; 5] = [10, 100, 1_000, 10_000, 100_000];

/// The mixed adversary's sizes for each new inner's own upstream, after the
/// switch in the order.
pub const NEW: [usize; 4] = [10, 100, 1_000, 10_000];

/// The mixed adversary's switch downstream, about what PK's forward search
/// covered on the earlier probe's UI graph.
pub const MIXED_DOWN: usize = 1_000;

/// Moves in an adversarial case, each to a new inner of its own.
pub const ADVERSARY_MOVES: usize = 16;

/// Appends a node depending on `deps`. The adversary's graphs are built
/// here, outside the earlier probe's generator, and only the walk, PK and
/// the lists run on them, which read no more than the edges.
fn add(g: &mut Graph, deps: &[Id]) -> Id {
    let n = g.len() as Id;
    g.deps.push(Vec::new());
    g.dependents.push(Vec::new());
    for &d in deps {
        g.link(d, n);
    }
    n
}

/// Appends `len` nodes downstream of `root`, each on the one or two before
/// it, so every one of them reaches the last and is reached from `root`,
/// and every path across them is at least half as long as they are: a
/// search that finds a cycle through them can't take a shortcut.
/// Returns the first and last.
fn web(g: &mut Graph, root: Id, len: usize) -> (Id, Id) {
    let first = g.len() as Id;
    for i in 0..len as Id {
        let n = first + i;
        match i {
            0 => add(g, &[root]),
            1 => add(g, &[n - 1]),
            _ => add(g, &[n - 1, n - 2]),
        };
    }
    (first, first + len as Id - 1)
}

/// A worst case for the order's searches: one switch, `down` nodes
/// downstream of it, moved [`ADVERSARY_MOVES`] times, each time to a new
/// inner whose `up` nodes upstream were all built after the switch, so sit
/// after it in the order, while the switch's downstream sits before the
/// new inner. The new inner is invalid against the order every time; the
/// backward search from it has `up + 1` nodes to cover and the forward
/// search from the switch `down + 1`.
///
/// Every new upstream is built before the first move, since only where a
/// node sits in the order matters here and a node built appends at the end
/// either way; so the moves can be measured without the builds. Moves go
/// from each inner to the next, and all are accepted.
///
/// `cyclic` hangs each new upstream off the switch's last downstream node
/// instead of an input, so every move closes a cycle through both sides
/// and is refused. The refused screen is then dropped, its edge from the
/// switch's downstream cut, as the engine would discard it, so that the
/// next move's forward search doesn't also cover it.
#[derive(Clone)]
pub struct Adversary<C> {
    pub run: Run<C>,
    pub moves: Vec<Move>,
    /// Cyclic only: the edge from the switch's downstream to each move's
    /// new upstream, cut once the move is refused.
    cuts: Vec<(Id, Id)>,
    old: Id,
    switch: Id,
    /// The checker's counts after warming up, before the first move.
    pub warm: Counts,
}

impl<C: Checker> Adversary<C> {
    pub fn new(up: usize, down: usize, cyclic: bool) -> Self {
        Self::build(0, up, down, cyclic)
    }

    /// The acyclic adversary, with every new inner also reading one shared
    /// upstream of `shared` nodes, a model, say, that the old inner reads
    /// too. It is built before the switch, and upstream of it, so it sits
    /// before the switch in any order: the walk covers it on every move, and
    /// neither list search goes into it. Each new inner's own upstream, `up`
    /// nodes after the switch, hangs off the shared upstream's last node.
    pub fn mixed(shared: usize, up: usize, down: usize) -> Self {
        assert!(shared > 0);
        Self::build(shared, up, down, false)
    }

    fn build(shared: usize, up: usize, down: usize, cyclic: bool) -> Self {
        assert!(up > 0 && down > 0);
        let mut g = Graph::default();
        let input = add(&mut g, &[]);
        // With no shared upstream, the graph is the one `new` always built.
        let (old, model) = if shared == 0 {
            (add(&mut g, &[]), input)
        } else {
            let (_, model) = web(&mut g, input, shared);
            (add(&mut g, &[model]), model)
        };
        let switch = add(&mut g, &[old]);
        let (_, last_down) = web(&mut g, switch, down);
        // A second switch, moved once below to warm up.
        let warm_old = add(&mut g, &[]);
        let warm_switch = add(&mut g, &[warm_old]);
        let mut run = Run {
            checker: C::new(&g),
            graph: g,
        };
        let first = run.graph.len() as Id;
        let (mut moves, mut cuts) = (Vec::new(), Vec::new());
        let mut from = old;
        for _ in 0..ADVERSARY_MOVES {
            let root = if cyclic { last_down } else { model };
            let (first_up, last_up) = web(&mut run.graph, root, up);
            let to = add(&mut run.graph, &[last_up]);
            let kind = if cyclic { Kind::Bad } else { Kind::View };
            moves.push(Move {
                switch,
                from,
                to,
                kind,
            });
            if cyclic {
                cuts.push((last_down, first_up));
            } else {
                from = to;
            }
        }
        let warm_new = add(&mut run.graph, &[]);
        run.checker.built(&run.graph, first);
        // The warm-up move is against the order, so every checker sizes its
        // visit stamps to the whole graph now rather than in the first
        // measured move, which would charge it to the moves.
        let warm = [Move {
            switch: warm_switch,
            from: warm_old,
            to: warm_new,
            kind: Kind::Local,
        }];
        assert!(run.checker.commit(&mut run.graph, &warm));
        let warm = run.checker.counts().clone();
        Adversary {
            run,
            moves,
            cuts,
            old,
            switch,
            warm,
        }
    }

    /// Commits move `k` alone; returns whether it was accepted.
    pub fn tx(&mut self, k: usize) -> bool {
        let run = &mut self.run;
        let ok = run.checker.commit(&mut run.graph, &self.moves[k..=k]);
        if !ok && let Some(&(from, to)) = self.cuts.get(k) {
            run.graph.unlink(from, to);
        }
        ok
    }

    /// Every move in turn; returns how many were refused.
    pub fn all(&mut self) -> usize {
        (0..self.moves.len()).filter(|&k| !self.tx(k)).count()
    }

    /// Puts the graph back as it was before the first move and the checker
    /// back to `fresh`, a clone of it taken then, so the moves can run
    /// again without rebuilding the graph.
    pub fn reset(&mut self, fresh: &C) {
        let g = &mut self.run.graph;
        let current = g.deps[self.switch as usize][0];
        if current != self.old {
            g.unlink(current, self.switch);
            g.link(self.old, self.switch);
        }
        for &(from, to) in &self.cuts {
            if !g.dependents[from as usize].contains(&to) {
                g.link(from, to);
            }
        }
        self.run.checker.clone_from(fresh);
    }

    /// Nodes the checker visited over the moves, apart from the warm-up.
    pub fn visited(&self) -> u64 {
        let now = self.run.checker.counts().visited.iter().sum::<u64>();
        now - self.warm.visited.iter().sum::<u64>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rfd_0005_bounded_relink_check::{
        Baseline, KINDS, Pk, Run, TXS, Workload, generate, workloads,
    };

    /// Whether every node in `seq` comes after its dependencies in `seq`.
    pub(super) fn topological_among(g: &Graph, seq: &[Id]) -> bool {
        let at: std::collections::HashMap<Id, usize> =
            seq.iter().enumerate().map(|(i, &n)| (n, i)).collect();
        seq.iter().enumerate().all(|(i, &n)| {
            g.deps[n as usize]
                .iter()
                .all(|d| at.get(d).is_none_or(|&j| j < i))
        })
    }

    /// Every edge goes forward in the order, and the list is sound.
    fn is_topological<const T: bool, const F: bool, const P: bool>(
        g: &Graph,
        c: &Om<T, F, P>,
    ) -> bool {
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
    fn f46_accepted<const T: bool, const F: bool, const P: bool>() {
        let w = generate(&workloads(4)[0].1);
        // Transaction 2 is F46's reversal, and 3 undoes it.
        let (there, back) = (&w.txs[2].moves, &w.txs[3].moves);
        assert!(there.iter().chain(back).all(|m| m.kind == Kind::F46));
        let mut run = Run::<Om<T, F, P>>::new(&w);
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
        f46_accepted::<false, false, false>();
        f46_accepted::<true, false, false>();
        f46_accepted::<false, true, false>();
        f46_accepted::<true, true, false>();
        f46_accepted::<false, true, true>();
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
                agrees::<BackFresh>(&w, is_topological);
                agrees::<TwoWayFresh>(&w, is_topological);
                agrees::<BackNoSort>(&w, is_topological);
            }
        }
    }

    /// Relabelling keeps the list in order when a gap runs out, with either
    /// spacing, moving nodes one at a time before node 1 or after node 0.
    #[test]
    fn relabelling_keeps_the_order() {
        for fresh in [false, true] {
            relabelling_keeps_the_order_with(fresh, false);
            relabelling_keeps_the_order_with(fresh, true);
        }
    }

    fn relabelling_keeps_the_order_with(fresh: bool, after: bool) {
        let mut o = Order::new(&[0, 1], 2);
        o.fresh = fresh;
        // Each node goes just before node 1, which halves the gap there
        // until it is gone.
        for n in 2..3002 {
            o.grow(n as usize + 1);
            o.push(n);
            if after {
                o.move_after(0, &[n]);
            } else {
                o.move_before(1, &[n]);
            }
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
        if after {
            want.extend((2..3002).rev());
        } else {
            want.extend(2..3002);
        }
        want.push(1);
        assert_eq!(seq, want);
    }

    /// Moving sets of many nodes into one gap, over and over, keeps the list
    /// in order with either spacing, and fresh spacing relabels less.
    #[test]
    fn fresh_spacing_keeps_the_order_and_relabels_less() {
        let mut relabeled = [0; 2];
        for (fresh, r) in [false, true].into_iter().zip(&mut relabeled) {
            for after in [false, true] {
                let mut o = Order::new(&[0, 1], 2);
                o.fresh = fresh;
                let mut len = 2;
                for k in (1..200).map(|i| 1 + i * 37 % 500) {
                    o.grow(len + k);
                    let set: Vec<Id> = (len as Id..(len + k) as Id).collect();
                    for &n in &set {
                        o.push(n);
                    }
                    if after {
                        o.move_after(0, &set);
                    } else {
                        o.move_before(1, &set);
                    }
                    len += k;
                    assert!(o.is_consistent(len));
                }
                *r += o.relabeled;
            }
        }
        assert!(relabeled[1] < relabeled[0], "{relabeled:?}");
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

    /// Runs one adversarial case through `C`, asserting `check` after every
    /// move, and returns it with its verdicts.
    fn adversary<C: Checker>(
        up: usize,
        down: usize,
        cyclic: bool,
        check: impl Fn(&Graph, &C) -> bool,
    ) -> (Adversary<C>, Vec<bool>) {
        moves(Adversary::<C>::new(up, down, cyclic), check)
    }

    /// Runs every move of `a`, asserting `check` before and after each.
    fn moves<C: Checker>(
        mut a: Adversary<C>,
        check: impl Fn(&Graph, &C) -> bool,
    ) -> (Adversary<C>, Vec<bool>) {
        assert!(check(&a.run.graph, &a.run.checker), "the order broke");
        let refused = (0..a.moves.len())
            .map(|k| {
                let refused = !a.tx(k);
                assert!(check(&a.run.graph, &a.run.checker), "the order broke");
                refused
            })
            .collect();
        (a, refused)
    }

    /// On the adversary too, every variant refuses exactly the moves the
    /// walk refuses, which is all of them when cyclic and none otherwise,
    /// and keeps a topological order; and a reset case runs the same again.
    #[test]
    fn every_variant_refuses_what_the_walk_refuses_on_the_adversary() {
        for (up, down) in [(1, 1), (10, 100), (100, 10), (37, 37)] {
            for cyclic in [false, true] {
                let want = vec![cyclic; ADVERSARY_MOVES];
                let (_, walk) = adversary::<Baseline>(up, down, cyclic, |_, _| true);
                assert_eq!(walk, want, "the adversary isn't what it says");
                let (_, pk) = adversary::<Pk>(up, down, cyclic, |_, _| true);
                assert_eq!(pk, walk, "pk differs from the walk");
                let (_, back) = adversary::<Back>(up, down, cyclic, is_topological);
                assert_eq!(back, walk, "back differs from the walk");
                let (_, two) = adversary::<TwoWay>(up, down, cyclic, is_topological);
                assert_eq!(two, walk, "twoway differs from the walk");
                let (_, bf) = adversary::<BackFresh>(up, down, cyclic, is_topological);
                assert_eq!(bf, walk, "back fresh differs from the walk");
                let (_, tf) = adversary::<TwoWayFresh>(up, down, cyclic, is_topological);
                assert_eq!(tf, walk, "twoway fresh differs from the walk");
                let (_, bn) = adversary::<BackNoSort>(up, down, cyclic, is_topological);
                assert_eq!(bn, walk, "back nosort differs from the walk");

                let mut a = Adversary::<TwoWay>::new(up, down, cyclic);
                let fresh = a.run.checker.clone();
                let once = (a.all(), a.visited());
                a.reset(&fresh);
                assert!(is_topological(&a.run.graph, &a.run.checker));
                assert_eq!((a.all(), a.visited()), once, "a reset case ran differently");
            }
        }
    }

    /// The adversary's counts: per move, the nodes each checker visited,
    /// and what the lists relabelled, for every pair of sizes. Run with
    /// `--nocapture`.
    #[test]
    fn adversarial_counts() {
        println!(
            "one switch, moved {ADVERSARY_MOVES} times, each to a new inner built after it; \
             up = the new inner's upstream, all after the switch in the order; \
             down = the switch's downstream, all before the new inner"
        );
        let mut worst_back = 0.0f64;
        let mut worst_two = 0.0f64;
        for cyclic in [false, true] {
            println!();
            if cyclic {
                println!(
                    "cyclic: each new upstream hangs off the switch's downstream, so every move is refused"
                );
            } else {
                println!("acyclic: every move accepted");
            }
            println!(
                "  {:>6} {:>6} {:>8} {:>8} {:>8} {:>8} {:>8} {:>9} {:>9} {:>9}",
                "up",
                "down",
                "walk",
                "pk",
                "back",
                "twoway",
                "2min+3",
                "tw/walk",
                "bk relab",
                "tw relab"
            );
            for up in SIDES {
                for down in SIDES {
                    let (walk, rw) = adversary::<Baseline>(up, down, cyclic, |_, _| true);
                    let (pk, rp) = adversary::<Pk>(up, down, cyclic, |_, _| true);
                    let (back, rb) = adversary::<Back>(up, down, cyclic, |_, _| true);
                    let (two, rt) = adversary::<TwoWay>(up, down, cyclic, |_, _| true);
                    assert!(rw.iter().all(|&r| r == cyclic));
                    assert!(rp == rw && rb == rw && rt == rw, "verdicts differ");
                    let per = |v: u64| v as f64 / ADVERSARY_MOVES as f64;
                    let (w, p, b, t) = (
                        per(walk.visited()),
                        per(pk.visited()),
                        per(back.visited()),
                        per(two.visited()),
                    );
                    let bound = 2 * up.min(down) + 3;
                    if !cyclic {
                        worst_back = worst_back.max(b / (up + 1) as f64);
                        worst_two = worst_two.max(t / bound as f64);
                    }
                    let (bl, tl) = (&back.run.checker.order, &two.run.checker.order);
                    println!(
                        "  {up:>6} {down:>6} {w:>8.1} {p:>8.1} {b:>8.1} {t:>8.1} {bound:>8} {:>9.2} {:>9.1} {:>9.1}",
                        t / w,
                        per(bl.relabeled),
                        per(tl.relabeled),
                    );
                }
            }
        }
        println!();
        println!("nodes visited per move: walked upstream from the new inner (walk), searched");
        println!(
            "forward and back (pk, twoway), or back only (back). 2min+3 is 2 min(up, down) + 3,"
        );
        println!("the most a two-way search that stops at the first side to finish can visit");
        println!("when the sides hold up + 1 and down + 1 nodes. Relab is nodes the list gave");
        println!("new labels to make room, per move, apart from those moved.");
        println!(
            "acyclic: back visited at most {worst_back:.2} times up + 1; twoway at most {worst_two:.2} times 2min+3"
        );
    }

    /// Sets moved into one gap in the list-alone rows.
    const LONG: usize = 1_000;

    /// Moves [`LONG`] sets of `k` new nodes each into the gap before node 1
    /// (or after node 0), checking the list after every move; returns the
    /// nodes relabelled.
    fn long_run(k: usize, after: bool, fresh: bool) -> u64 {
        let mut o = Order::new(&[0, 1], 2);
        o.fresh = fresh;
        let mut len = 2;
        let mut set = Vec::with_capacity(k);
        for _ in 0..LONG {
            o.grow(len + k);
            set.clear();
            set.extend(len as Id..(len + k) as Id);
            for &n in &set {
                o.push(n);
            }
            if after {
                o.move_after(0, &set);
            } else {
                o.move_before(1, &set);
            }
            len += k;
            assert!(o.is_consistent(len));
        }
        o.relabeled
    }

    /// Relabelled nodes per move, apart from those moved.
    fn relabels<const T: bool, const F: bool, const P: bool>(c: &Om<T, F, P>, moves: u64) -> f64 {
        c.order.relabeled as f64 / moves as f64
    }

    /// Mixed adversarial and fresh-spacing counts: what fresh spacing
    /// relabels against the even spacing, on the earlier probe's workloads
    /// and on the adversary, and the nodes each checker visits on the mixed
    /// adversary. Every list's order is checked after every transaction or
    /// move. Run with `--nocapture`.
    #[test]
    fn mixed_spacing_counts() {
        println!("fresh spacing: a moved set takes half its gap, away from where the next set");
        println!("to that spot goes, and a relabelling leaves half its range open there");
        println!();
        println!("{TXS} transactions per workload, 2 slots each; per move:");
        println!(
            "  {:<8} {:>6} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}",
            "workload",
            "moves",
            "bk vis",
            "bf vis",
            "tw vis",
            "tf vis",
            "bk relab",
            "bf relab",
            "tw relab",
            "tf relab"
        );
        for (name, p) in workloads(TXS) {
            let w = generate(&p);
            let back = agrees::<Back>(&w, is_topological);
            let bf = agrees::<BackFresh>(&w, is_topological);
            let two = agrees::<TwoWay>(&w, is_topological);
            let tf = agrees::<TwoWayFresh>(&w, is_topological);
            let moves: u64 = back.checker.counts().moves.iter().sum();
            let vis = |c: &Counts| c.visited.iter().sum::<u64>() as f64 / moves as f64;
            println!(
                "  {name:<8} {moves:>6} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8.1}",
                vis(back.checker.counts()),
                vis(bf.checker.counts()),
                vis(two.checker.counts()),
                vis(tf.checker.counts()),
                relabels(&back.checker, moves),
                relabels(&bf.checker, moves),
                relabels(&two.checker, moves),
                relabels(&tf.checker, moves),
            );
        }

        println!();
        println!(
            "the adversary, acyclic ({ADVERSARY_MOVES} moves, all accepted; no cyclic move is \
             reordered, so none relabels); per move:"
        );
        println!(
            "  {:>6} {:>6} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}",
            "up", "down", "back", "twoway", "bk relab", "bf relab", "tw relab", "tf relab"
        );
        let per = |v: u64| v as f64 / ADVERSARY_MOVES as f64;
        for up in SIDES {
            for down in SIDES {
                let (back, rb) = adversary::<Back>(up, down, false, is_topological);
                let (bf, rbf) = adversary::<BackFresh>(up, down, false, is_topological);
                let (two, rt) = adversary::<TwoWay>(up, down, false, is_topological);
                let (tf, rtf) = adversary::<TwoWayFresh>(up, down, false, is_topological);
                assert!([rb, rbf, rt, rtf].iter().flatten().all(|&r| !r));
                assert_eq!(back.visited(), bf.visited(), "back's search changed");
                assert_eq!(two.visited(), tf.visited(), "twoway's search changed");
                let relab = |o: &Order| per(o.relabeled);
                println!(
                    "  {up:>6} {down:>6} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8.1}",
                    per(back.visited()),
                    per(two.visited()),
                    relab(&back.run.checker.order),
                    relab(&bf.run.checker.order),
                    relab(&two.run.checker.order),
                    relab(&tf.run.checker.order),
                );
            }
        }

        println!();
        println!(
            "mixed adversary: {ADVERSARY_MOVES} moves, all accepted, each to a new inner reading \
             `shared` nodes before the switch in the order and `new` nodes of its own after it; \
             the switch's downstream is {MIXED_DOWN}; per move:"
        );
        println!(
            "  {:>6} {:>6} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}",
            "shared",
            "new",
            "walk",
            "back",
            "twoway",
            "bk relab",
            "bf relab",
            "tw relab",
            "tf relab"
        );
        for shared in SHARED {
            for new in NEW {
                let (walk, rw) = moves(
                    Adversary::<Baseline>::mixed(shared, new, MIXED_DOWN),
                    |_, _| true,
                );
                let (back, rb) = moves(
                    Adversary::<Back>::mixed(shared, new, MIXED_DOWN),
                    is_topological,
                );
                let (bf, rbf) = moves(
                    Adversary::<BackFresh>::mixed(shared, new, MIXED_DOWN),
                    is_topological,
                );
                let (two, rt) = moves(
                    Adversary::<TwoWay>::mixed(shared, new, MIXED_DOWN),
                    is_topological,
                );
                let (tf, rtf) = moves(
                    Adversary::<TwoWayFresh>::mixed(shared, new, MIXED_DOWN),
                    is_topological,
                );
                assert!([rw, rb, rbf, rt, rtf].iter().flatten().all(|&r| !r));
                assert_eq!(back.visited(), bf.visited(), "back's search changed");
                assert_eq!(two.visited(), tf.visited(), "twoway's search changed");
                let relab = |o: &Order| per(o.relabeled);
                println!(
                    "  {shared:>6} {new:>6} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8.1}",
                    per(walk.visited()),
                    per(back.visited()),
                    per(two.visited()),
                    relab(&back.run.checker.order),
                    relab(&bf.run.checker.order),
                    relab(&two.run.checker.order),
                    relab(&tf.run.checker.order),
                );
            }
        }
        println!();
        println!(
            "the list alone: {LONG} sets of k new nodes, each moved just before one node \
             (as back does) or just after one (as twoway's forward side does); per move:"
        );
        println!(
            "  {:>6} {:>12} {:>12} {:>12} {:>12}",
            "k", "before even", "before fresh", "after even", "after fresh"
        );
        for k in [1, 10, 100, 1_000] {
            let mut row = [0.0; 4];
            for (j, (after, fresh)) in [(false, false), (false, true), (true, false), (true, true)]
                .into_iter()
                .enumerate()
            {
                row[j] = long_run(k, after, fresh) as f64 / LONG as f64;
            }
            println!(
                "  {k:>6} {:>12.1} {:>12.1} {:>12.1} {:>12.1}",
                row[0], row[1], row[2], row[3]
            );
        }
        println!();
        println!("vis and walk/back/twoway: nodes visited per move, walked upstream from the new");
        println!("inner (walk), searched back only (back, bk, bf) or both ways (twoway, tw, tf).");
        println!(
            "bf and tf are back and twoway with fresh spacing. relab: nodes the list gave new"
        );
        println!("labels to make room, per move, apart from those moved. On the adversaries the");
        println!("searches visit the same nodes with either spacing; on the workloads the orders");
        println!("differ, so the searches can too.");
    }

    /// The share of `c`'s moved sets that were topological among themselves
    /// as found and reversed, as percentages, and how many there were.
    fn discovery<const T: bool, const F: bool, const P: bool>(c: &Om<T, F, P>) -> (u64, f64, f64) {
        let [sets, found, rev] = c.discovery;
        let pc = |k: u64| 100.0 * k as f64 / sets.max(1) as f64;
        (sets, pc(found), pc(rev))
    }

    /// The no-sort counts: nodes visited and relabelled by `back` with fresh
    /// spacing against the same with a depth-first search moving its set in
    /// post-order, on the earlier probe's workloads, the adversary and the
    /// mixed adversary; and whether the stack search's own order, or its
    /// reverse, was already topological among the set. Every list's order is
    /// checked after every transaction or move. Run with `--nocapture`.
    #[test]
    fn nosort_counts() {
        println!("bf: back with fresh spacing, its stack search's set sorted by label before");
        println!("the move; bn: the same with a depth-first search, its set moved in post-order");
        println!();
        println!("{TXS} transactions per workload, 2 slots each; per move:");
        println!(
            "  {:<8} {:>6} {:>8} {:>8} {:>8} {:>8} {:>7} {:>8} {:>8}",
            "workload",
            "moves",
            "bf vis",
            "bn vis",
            "bf relab",
            "bn relab",
            "sets",
            "found ok",
            "rev ok"
        );
        for (name, p) in workloads(TXS) {
            let w = generate(&p);
            let bf = agrees::<BackFresh>(&w, is_topological);
            let bn = agrees::<BackNoSort>(&w, is_topological);
            let moves: u64 = bf.checker.counts().moves.iter().sum();
            let vis = |c: &Counts| c.visited.iter().sum::<u64>() as f64 / moves as f64;
            let (sets, found, rev) = discovery(&bf.checker);
            println!(
                "  {name:<8} {moves:>6} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {sets:>7} {found:>7.1}% {rev:>7.1}%",
                vis(bf.checker.counts()),
                vis(bn.checker.counts()),
                relabels(&bf.checker, moves),
                relabels(&bn.checker, moves),
            );
        }

        let per = |v: u64| v as f64 / ADVERSARY_MOVES as f64;
        for cyclic in [false, true] {
            println!();
            if cyclic {
                println!(
                    "the adversary, cyclic ({ADVERSARY_MOVES} moves, all refused, none moved); per move:"
                );
            } else {
                println!(
                    "the adversary, acyclic ({ADVERSARY_MOVES} moves, all accepted); per move:"
                );
            }
            println!(
                "  {:>6} {:>6} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}",
                "up",
                "down",
                "walk",
                "bf vis",
                "bn vis",
                "bf relab",
                "bn relab",
                "found ok",
                "rev ok"
            );
            for up in SIDES {
                for down in SIDES {
                    let (walk, rw) = adversary::<Baseline>(up, down, cyclic, |_, _| true);
                    let (bf, rbf) = adversary::<BackFresh>(up, down, cyclic, is_topological);
                    let (bn, rbn) = adversary::<BackNoSort>(up, down, cyclic, is_topological);
                    assert!(
                        [&rw, &rbf, &rbn]
                            .iter()
                            .all(|r| r.iter().all(|&r| r == cyclic))
                    );
                    let (bfc, bnc) = (&bf.run.checker, &bn.run.checker);
                    // The warm-up move is one set, of its new inner alone,
                    // so always topological: it is left out.
                    let [sets, found, rev] = bfc.discovery;
                    assert_eq!(sets, 1 + if cyclic { 0 } else { ADVERSARY_MOVES as u64 });
                    assert!(found >= 1 && rev >= 1);
                    let ok = |k: u64| match sets - 1 {
                        0 => "-".to_string(),
                        n => format!("{:.1}%", 100.0 * (k - 1) as f64 / n as f64),
                    };
                    println!(
                        "  {up:>6} {down:>6} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8} {:>8}",
                        per(walk.visited()),
                        per(bf.visited()),
                        per(bn.visited()),
                        per(bfc.order.relabeled),
                        per(bnc.order.relabeled),
                        ok(found),
                        ok(rev),
                    );
                }
            }
        }

        println!();
        println!(
            "mixed adversary: {ADVERSARY_MOVES} moves, all accepted, each to a new inner reading \
             `shared` nodes before the switch in the order and `new` nodes of its own after it; \
             the switch's downstream is {MIXED_DOWN}; per move:"
        );
        println!(
            "  {:>6} {:>6} {:>8} {:>8} {:>8} {:>8} {:>8}",
            "shared", "new", "walk", "bf vis", "bn vis", "bf relab", "bn relab"
        );
        for shared in SHARED {
            for new in NEW {
                let (walk, rw) = moves(
                    Adversary::<Baseline>::mixed(shared, new, MIXED_DOWN),
                    |_, _| true,
                );
                let (bf, rbf) = moves(
                    Adversary::<BackFresh>::mixed(shared, new, MIXED_DOWN),
                    is_topological,
                );
                let (bn, rbn) = moves(
                    Adversary::<BackNoSort>::mixed(shared, new, MIXED_DOWN),
                    is_topological,
                );
                assert!([rw, rbf, rbn].iter().flatten().all(|&r| !r));
                println!(
                    "  {shared:>6} {new:>6} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8.1}",
                    per(walk.visited()),
                    per(bf.visited()),
                    per(bn.visited()),
                    per(bf.run.checker.order.relabeled),
                    per(bn.run.checker.order.relabeled),
                );
            }
        }
        println!();
        println!("vis and walk: nodes visited per move, walked upstream from the new inner");
        println!("(walk) or searched back: expanded by bf's stack search, entered by bn's");
        println!("depth-first one. relab: nodes the list gave new labels to make room, per");
        println!("move, apart from those moved. sets: sets bf moved before a switch; found ok");
        println!("and rev ok: the share of them that were topological among themselves in the");
        println!("order bf's search found them, and in the reverse, so could have moved without");
        println!("a sort (on the adversary, the warm-up move left out; - where none moved).");
    }
}
