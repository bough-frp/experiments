//! Do Incremental's small-integer heights, raised at link time and never
//! lowered, win back the bucket queue's advantage on quiet regions once
//! switches move, with the raises and the cycle check counted?
//!
//! `rfd-0005-heap-vs-mark-on-quiet-regions` found a bucket queue by height
//! beats RFD 5's mark plus flat loop from about 16% quiet, with static
//! heights. `rfd-0005-maintained-rank-queue` kept the order under switching
//! as an order-maintenance list, whose sparse `u64` labels need a heap, and
//! the heap cost 2.1 to 2.7 times the mark when everything fires. Jane
//! Street's Incremental keeps neither a dense order nor sparse labels: a
//! node's *height* is a small integer above every dependency's, so a bucket
//! per height works (incremental@v0.17.0 `incremental_intf.ml:111-139`,
//! quoted in synthesis 14).
//!
//! [`Heights`] is that scheme as `adjust_heights_heap.mli:3-8, 39-69`
//! describes it. Linking `child → parent` with `height(child) >=
//! height(parent)` raises the parent to `height(child) + 1` and puts it in
//! an adjust heap keyed by its height *before* the raise; nodes leave the
//! heap in increasing pre-raise height, and each raises its dependents the
//! same way. If a dependent reached that way is the original child, the
//! link closed a cycle and the adjustment stops. Heights are never lowered:
//! an unlink changes nothing. A node built gets one more than its highest
//! dependency, and nothing existing moves, since nothing depends on it yet.
//! Incremental caps heights at `max_height_allowed`, 128 by default, and
//! raises beyond it; the cap isn't enforced here, and the counts report the
//! heights reached.
//!
//! Around the adjustment, the commit is the other checkers': the
//! transaction's unlinks first, then each new link adjusted in turn, so
//! F46's reversal is never refused. On a refusal the new links are undone,
//! the old relinked, and the adjust heap drained over the restored graph, so
//! the raises the cycle left behind still hold every edge. Incremental
//! instead treats a cycle as fatal; Bough poisons, and the workload must go
//! on.
//!
//! [`Buckets`] evaluates through a bucket per height: the input's
//! dependents pushed, then a cursor climbing from the input's height pops
//! each bucket, evaluates the node, and pushes its dependents only when it
//! fired, once each per transaction. A dependent is always higher, so the
//! cursor never goes back, and it stops at the highest bucket pushed to.
//!
//! The graph, workloads, events, node semantics and the other engines are
//! `rfd-0005-maintained-rank-queue`'s, used through its public API;
//! [`agree`] holds all five engines to the same fired set, values and
//! verdict on every transaction.
//!
//! [`Heights`] also has a variant without the original-child check
//! (`GUARD = false`), where the only thing that stops a cycle's raises going
//! round forever is the height cap. The counts use it to say what that
//! detector costs and whether it refuses exactly what the walk refuses.
//!
//! All of that leaves out the two cases RFD 5 names as why rank-ordered
//! push was rejected: nodes built during the instant run in it, after what
//! they depend on, and a `switch_cell` at a switch instant emits its new
//! inner's post-instant value, a dependency found mid-transaction. The
//! second half of the file puts them in. A transaction's closure runs, and
//! its moves become known, when its construct point is evaluated: the root
//! event stream's forward token, which the selectors read, forced into the
//! region (visited, firing only if its dependency did); for F46's pair,
//! whose selector is off the tree, the start. A `switch_cell` that moves
//! fires whatever its inner did, with the new inner's post-instant value; a
//! `switch_stream` keeps its old inner for the instant and moves at commit.
//! A refused transaction poisons: none of its values commit.
//!
//! - [`MarkPull`]: RFD 5 with its fallback. The mark runs from the input
//!   and the construct point; built nodes are pulled once the closure
//!   returns, and a moving `switch_cell` pulls its new inner when the loop
//!   reaches it, so the loop checks a memo stamp before each node, and a
//!   pull that comes back to the switch is a cycle.
//! - [`HeightsAt`]: at the construct point the built nodes get heights,
//!   and the moving `switch_cell`s are relinked with Incremental's raise,
//!   which finds a cycle as at commit. A switch always comes after its
//!   selector, so a raise never reaches below the cursor; a built node can
//!   land there (an event reading an input), and then either the cursor is
//!   re-seated down to it (`RESEAT`) or it is evaluated at once, out of the
//!   height order.
//! - [`MarkUnforced`]: [`MarkPull`] with nothing forced into the mark.
//!   Forcing the construct point marks everything downstream of the root
//!   event stream, every view, where the heights visit only what fires; this
//!   one marks from the input and the moving `switch_cell`s alone, runs the
//!   closure at the start and pulls what it built, so the comparison with
//!   the heights is of scheduling, not of region.
//! - [`Recompute`]: every node evaluated in a fresh topological order,
//!   the reference [`agree_instant`] holds the others to.

use crate::rfd_0005_bounded_relink_check::{
    Baseline, Checker, Counts, Graph, Id, Kind, Move, Run, Tx,
};
use crate::rfd_0005_maintained_rank_queue::{
    Engine, Fixture, HeapQueue, MarkOm, Outcome, RadixQueue, Schedule, State, Walked,
};
use crate::rfd_0005_small_side_order::TwoWay;

pub use crate::rfd_0005_maintained_rank_queue::{PASS, WORKLOADS};

/// Not in the adjust heap.
const OUT: u32 = u32::MAX;

/// What the height upkeep did, beyond [`Counts`]' per-kind moves
/// (`moves`), links that needed a raise (`invalid`) and nodes the raises
/// touched (`visited`).
#[derive(Clone, Default, Debug)]
pub struct Stats {
    /// Links adjusted at commit: every new inner.
    pub links: u64,
    /// Of those, links whose child was not already below the parent.
    pub raises: u64,
    /// Nodes taken out of the adjust heap: each had been raised, and had
    /// its dependents checked.
    pub touched: u64,
    /// Dependent edges checked from those nodes.
    pub edges: u64,
    /// The most nodes one link's adjustment touched.
    pub largest: u64,
    /// Nodes touched after a refusal, finishing the interrupted raises and
    /// relinking the old inners.
    pub restored: u64,
    /// Nodes given a height as they were built.
    pub built: u64,
    /// Transactions refused.
    pub refused: u64,
}

/// Incremental's heights, with its adjust heap. `GUARD` is the
/// original-child check; without it only `cap` stops a cycle.
#[derive(Clone)]
pub struct Heights<const GUARD: bool = true> {
    pub height: Vec<u32>,
    /// The adjust heap: a bucket per pre-raise height, and each node's key
    /// in it, or [`OUT`].
    buckets: Vec<Vec<Id>>,
    key: Vec<u32>,
    lo: usize,
    len: usize,
    /// A raise above this is refused as a cycle (`GUARD = false` only).
    pub cap: u32,
    /// The largest height any node has had.
    pub max: u32,
    pub stats: Stats,
    counts: Counts,
}

impl<const GUARD: bool> Heights<GUARD> {
    /// Heights from scratch: the longest path from a node with no
    /// dependencies, by Kahn's algorithm.
    pub fn from_scratch(g: &Graph) -> Vec<u32> {
        let mut indeg: Vec<u32> = g.deps.iter().map(|d| d.len() as u32).collect();
        let mut order: Vec<Id> = (0..g.len() as Id)
            .filter(|&n| indeg[n as usize] == 0)
            .collect();
        let mut height = vec![0; g.len()];
        let mut k = 0;
        while k < order.len() {
            let n = order[k];
            k += 1;
            for &d in &g.dependents[n as usize] {
                height[d as usize] = height[d as usize].max(height[n as usize] + 1);
                indeg[d as usize] -= 1;
                if indeg[d as usize] == 0 {
                    order.push(d);
                }
            }
        }
        assert_eq!(order.len(), g.len(), "the graph is acyclic");
        height
    }

    /// Whether every edge goes from a lower height to a higher one.
    pub fn valid(&self, g: &Graph) -> bool {
        (0..g.len()).all(|n| {
            g.deps[n]
                .iter()
                .all(|&d| self.height[d as usize] < self.height[n])
        })
    }

    /// Puts `n` in the adjust heap, keyed by its height now.
    fn enqueue(&mut self, n: Id) {
        let k = self.height[n as usize];
        self.key[n as usize] = k;
        if self.buckets.len() <= k as usize {
            self.buckets.resize(k as usize + 1, Vec::new());
        }
        self.buckets[k as usize].push(n);
        self.lo = self.lo.min(k as usize);
        self.len += 1;
    }

    /// Raises `parent` above `child` if it isn't, keying it in the heap by
    /// its height before the first raise. False if the raise passes the cap.
    #[inline]
    fn ensure(&mut self, child: Id, parent: Id) -> bool {
        let need = self.height[child as usize] + 1;
        let p = parent as usize;
        if self.height[p] >= need {
            return true;
        }
        if self.key[p] == OUT {
            self.enqueue(parent);
        }
        self.height[p] = need;
        self.max = self.max.max(need);
        GUARD || need <= self.cap
    }

    fn pop(&mut self) -> Option<Id> {
        if self.len == 0 {
            self.lo = usize::MAX;
            return None;
        }
        while self.buckets[self.lo].is_empty() {
            self.lo += 1;
        }
        self.len -= 1;
        let n = self.buckets[self.lo].pop().expect("non-empty");
        self.key[n as usize] = OUT;
        Some(n)
    }

    /// Empties the adjust heap in increasing pre-raise height, raising each
    /// node's dependents. False when a dependent is `original` (with
    /// `GUARD`) or a raise passes the cap, leaving the rest in the heap and
    /// the node that stopped back in it, since not all its dependents were
    /// checked. A node raised after it left the heap goes back in, so the
    /// order is only for speed: whatever order the heap is drained in, it
    /// ends with every edge held.
    fn drain(&mut self, g: &Graph, original: Id, touched: &mut u64) -> bool {
        while let Some(n) = self.pop() {
            *touched += 1;
            for &p in &g.dependents[n as usize] {
                self.stats.edges += 1;
                if (GUARD && p == original) || !self.ensure(n, p) {
                    self.enqueue(n);
                    return false;
                }
            }
        }
        true
    }

    /// Links `child → parent`, raising what it must; false on a cycle.
    fn link(&mut self, g: &mut Graph, child: Id, parent: Id, kind: usize) -> bool {
        g.link(child, parent);
        self.stats.links += 1;
        if self.height[child as usize] < self.height[parent as usize] {
            return true;
        }
        self.stats.raises += 1;
        self.counts.invalid[kind] += 1;
        let mut touched = 0;
        let ok = self.ensure(child, parent) && self.drain(g, child, &mut touched);
        self.stats.touched += touched;
        self.stats.largest = self.stats.largest.max(touched);
        self.counts.visited[kind] += touched;
        ok
    }

    /// After a refusal, with the old graph back: finishes the raises the
    /// cycle interrupted, then relinks the old inners' heights. The old
    /// graph is acyclic, so none of it can fail.
    fn restore(&mut self, g: &Graph, moves: &[Move]) {
        if !GUARD {
            // Without the check, the cycle's raises ran up to the cap and
            // left heights there that would refuse the next raise through
            // them; start the heights again rather than inherit that.
            for b in &mut self.buckets {
                b.clear();
            }
            self.key.iter_mut().for_each(|k| *k = OUT);
            self.len = 0;
            self.lo = usize::MAX;
            self.height = Self::from_scratch(g);
            return;
        }
        let mut touched = 0;
        let drained = self.drain(g, Id::MAX, &mut touched);
        debug_assert!(drained);
        for m in moves {
            if !self.ensure(m.from, m.switch) || !self.drain(g, Id::MAX, &mut touched) {
                unreachable!("the old graph is acyclic");
            }
        }
        self.stats.restored += touched;
    }
}

impl<const GUARD: bool> Checker for Heights<GUARD> {
    fn new(g: &Graph) -> Self {
        let height = Self::from_scratch(g);
        let max = height.iter().copied().max().unwrap_or(0);
        Heights {
            buckets: vec![Vec::new(); max as usize + 1],
            key: vec![OUT; g.len()],
            height,
            lo: usize::MAX,
            len: 0,
            cap: u32::MAX,
            max,
            stats: Stats::default(),
            counts: Counts::default(),
        }
    }

    /// A node built depends only on older ones, and nothing on it yet.
    fn built(&mut self, g: &Graph, first: Id) {
        for n in first as usize..g.len() {
            let h = g.deps[n]
                .iter()
                .map(|&d| self.height[d as usize] + 1)
                .max()
                .unwrap_or(0);
            self.height.push(h);
            self.key.push(OUT);
            self.max = self.max.max(h);
            self.stats.built += 1;
        }
    }

    fn commit(&mut self, g: &mut Graph, moves: &[Move]) -> bool {
        for m in moves {
            self.counts.moves[m.kind as usize] += 1;
        }
        // Unlinks first: they never break a height, and after them each
        // graph on the way is a subgraph of the final one (F46).
        for m in moves {
            g.unlink(m.from, m.switch);
        }
        let mut linked = 0;
        let mut ok = true;
        while ok && linked < moves.len() {
            let m = moves[linked];
            linked += 1;
            ok = self.link(g, m.to, m.switch, m.kind as usize);
        }
        if ok {
            return true;
        }
        self.counts.refused += 1;
        self.stats.refused += 1;
        for m in &moves[..linked] {
            g.unlink(m.to, m.switch);
        }
        for m in moves {
            g.link(m.from, m.switch);
        }
        self.restore(g, moves);
        false
    }

    fn counts(&self) -> &Counts {
        &self.counts
    }
}

/// A bucket per height, popped by one climbing cursor; a node's dependents
/// are pushed only when it fires, once each per transaction.
#[derive(Clone, Default)]
pub struct Buckets {
    buckets: Vec<Vec<Id>>,
    queued: Vec<u32>,
    tx: u32,
    popped: u32,
    /// Empty buckets the last transaction's cursor stepped over.
    pub stepped: u32,
}

impl Buckets {
    #[inline]
    fn push_dependents(&mut self, g: &Graph, height: &[u32], n: Id, mut top: usize) -> usize {
        for &d in &g.dependents[n as usize] {
            if self.queued[d as usize] != self.tx {
                self.queued[d as usize] = self.tx;
                let h = height[d as usize] as usize;
                self.buckets[h].push(d);
                top = top.max(h);
            }
        }
        top
    }
}

impl<const GUARD: bool> Schedule<Heights<GUARD>> for Buckets {
    fn evaluate(&mut self, st: &mut State, run: &Run<Heights<GUARD>>, input: Id) {
        let g = &run.graph;
        let height = &run.checker.height;
        self.tx += 1;
        self.queued.resize(g.len(), 0);
        if self.buckets.len() <= run.checker.max as usize {
            self.buckets
                .resize(run.checker.max as usize + 1, Vec::new());
        }
        self.popped = 0;
        self.stepped = 0;
        let mut h = height[input as usize] as usize + 1;
        let mut top = self.push_dependents(g, height, input, 0);
        while h <= top {
            let Some(n) = self.buckets[h].pop() else {
                h += 1;
                self.stepped += 1;
                continue;
            };
            self.popped += 1;
            if st.eval(g, n) {
                top = self.push_dependents(g, height, n, top);
            }
        }
    }

    fn visited(&self) -> u32 {
        self.popped
    }
}

/// Incremental's heights and a bucket queue by them.
pub type HeightQueue = Engine<Heights, Buckets>;

/// Runs every engine over the fixture and panics unless each transaction
/// fired the same nodes with the same values and got the same verdict, the
/// heights hold every edge after each (in debug builds), and the committed
/// values end the same.
pub fn agree(f: &Fixture) {
    let mut walked = Walked::new(f);
    let mut mark_om = MarkOm::new(f);
    let mut radix = RadixQueue::new(f);
    let mut heap = HeapQueue::new(f);
    let mut heights = HeightQueue::new(f);
    for (k, (t, &e)) in f.txs.iter().zip(&f.events).enumerate() {
        let o = walked.tx(t, e);
        let others = [
            mark_om.tx(t, e),
            radix.tx(t, e),
            heap.tx(t, e),
            heights.tx(t, e),
        ];
        assert!(others.iter().all(|x| *x == o), "tx {k}: {o:?} {others:?}");
        debug_assert!(heights.run.checker.valid(&heights.run.graph), "tx {k}");
    }
    let v = walked.values();
    assert!(
        mark_om.values() == v && radix.values() == v && heap.values() == v && heights.values() == v
    );
}

// The instant with its two dynamic cases.

/// The root event stream's forward token: the relink probe's generator
/// builds 64 inputs and then it. The state holds read it and every
/// component's lifts read the state, so it is upstream of every view and
/// every switch over views, and below all of them in height.
pub const FORWARD: Id = 64;

/// A few multiply-xors, as the maintained-rank probe's `mix`.
#[inline]
fn mix(a: u64, b: u64) -> u64 {
    let mut x = a ^ b.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 32;
    x = x.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    x ^ (x >> 32)
}

/// Where a transaction's builds happen and its moves become known: when
/// the root event stream is evaluated, for navigations and local switches,
/// whose selectors read it; at the start for F46's pair, whose selector is
/// off the component tree. `None` is the start.
#[inline]
fn construct_point(t: &Tx) -> Option<Id> {
    if t.moves.iter().any(|m| m.kind == Kind::F46) {
        None
    } else {
        Some(FORWARD)
    }
}

/// A `switch_cell`'s move, as opposed to a `switch_stream`'s: at the switch
/// instant it reads its new inner's post-instant value.
#[inline]
fn is_cell(m: &Move) -> bool {
    m.kind != Kind::Event
}

/// What the instant engines did, summed over their transactions.
#[derive(Clone, Default, Debug)]
pub struct InstantStats {
    /// Nodes marked (mark and pull), or popped (heights).
    pub visited: u64,
    /// Nodes built during the transactions.
    pub built: u64,
    /// Mark and pull: built nodes evaluated by pull once the closure
    /// returned.
    pub pulled: u64,
    /// Mark and pull: existing nodes pulled ahead of the flat loop, by a
    /// built node or a `switch_cell`'s new inner.
    pub pulled_early: u64,
    /// Heights: built nodes at or below the cursor when they were built.
    pub below: u64,
    /// Heights, pulling: of those, the ones with a fired dependency,
    /// evaluated out of order as they were built.
    pub out_of_order: u64,
    /// Heights, re-seating: times the cursor went back down to a built node.
    pub reseats: u64,
    /// Heights: empty buckets the cursor stepped over.
    pub stepped: u64,
    /// Heights: queued nodes a raise moved up, popped at the old height
    /// and pushed again at the new one.
    pub stale: u64,
    /// Heights: `switch_cell` links made mid-evaluation that needed a raise,
    /// and the nodes those raises touched.
    pub raises: u64,
    pub touched: u64,
    /// Transactions found to close a same-instant cycle during evaluation.
    pub poisoned: u64,
}

/// The nodes' slots for the instant engines: the maintained-rank probe's
/// `State`, whose fields are private to it, with what a switching
/// `switch_cell` reads.
#[derive(Clone)]
pub struct Slots {
    value: Vec<u64>,
    stamp: Vec<u32>,
    current: Vec<u64>,
    /// A `switch_cell` switching at `switching[n] == tx` reads `to[n]`.
    switching: Vec<u32>,
    to: Vec<Id>,
    tx: u32,
    fired: Vec<Id>,
    digest: u64,
    threshold: u32,
}

impl Slots {
    fn new(len: usize, pass: u32) -> Slots {
        let mut me = Slots {
            value: Vec::new(),
            stamp: Vec::new(),
            current: Vec::new(),
            switching: Vec::new(),
            to: Vec::new(),
            tx: 0,
            fired: Vec::new(),
            digest: 0,
            threshold: pass * 1024 / 100,
        };
        me.grow(len);
        me
    }

    /// Slots for nodes built since, with a committed value each.
    fn grow(&mut self, len: usize) {
        let old = self.current.len();
        self.value.resize(len, 0);
        self.stamp.resize(len, 0);
        self.switching.resize(len, 0);
        self.to.resize(len, 0);
        self.current.extend((old..len).map(|n| mix(n as u64, 0)));
    }

    fn begin(&mut self, input: Id, value: u64) {
        self.tx += 1;
        self.fired.clear();
        self.digest = 0;
        self.value[input as usize] = value;
        self.fire(input);
    }

    #[inline]
    fn has_fired(&self, n: Id) -> bool {
        self.stamp[n as usize] == self.tx
    }

    #[inline]
    fn fire(&mut self, n: Id) {
        let i = n as usize;
        self.stamp[i] = self.tx;
        self.fired.push(n);
        self.digest = self.digest.wrapping_add(mix(self.value[i], n as u64));
    }

    #[inline]
    fn switch_to(&mut self, switch: Id, to: Id) {
        self.switching[switch as usize] = self.tx;
        self.to[switch as usize] = to;
    }

    #[inline]
    fn is_switching(&self, n: Id) -> bool {
        self.switching[n as usize] == self.tx
    }

    /// The maintained-rank probe's evaluation, except that a `switch_cell`
    /// switching now fires whatever its inner did, with its new inner's
    /// post-instant value: the value the generic code gives a node whose
    /// one dependency is that inner and fired.
    #[inline]
    pub fn eval(&mut self, g: &Graph, n: Id) -> bool {
        if self.is_switching(n) {
            let d = self.to[n as usize];
            let v = if self.has_fired(d) {
                self.value[d as usize]
            } else {
                self.current[d as usize]
            };
            self.value[n as usize] = mix(mix(v, d as u64), n as u64);
            self.fire(n);
            return true;
        }
        let deps = &g.deps[n as usize];
        if !deps.iter().any(|&d| self.has_fired(d)) {
            return deps.is_empty() && self.has_fired(n);
        }
        let mut acc = 0u64;
        let mut filter = false;
        for &d in deps {
            let v = if self.has_fired(d) {
                self.value[d as usize]
            } else {
                self.current[d as usize]
            };
            acc = acc.wrapping_add(mix(v, d as u64));
            filter |= d <= FORWARD;
        }
        let v = mix(acc, n as u64);
        if filter && (v >> 54) as u32 >= self.threshold {
            return false;
        }
        self.value[n as usize] = v;
        self.fire(n);
        true
    }

    /// Commits the fired values if the transaction was accepted. A refused
    /// one poisons in the engine; here it commits nothing and reports
    /// nothing fired, whatever it had evaluated when it stopped.
    fn finish(&mut self, accepted: bool) -> Outcome {
        if !accepted {
            return Outcome {
                fired: 0,
                digest: 0,
                accepted,
            };
        }
        for &n in &self.fired {
            self.current[n as usize] = self.value[n as usize];
        }
        Outcome {
            fired: self.fired.len() as u32,
            digest: self.digest,
            accepted,
        }
    }

    /// The committed values, to compare engines after a run.
    pub fn values(&self) -> &[u64] {
        &self.current
    }
}

/// A pull reached a node already being pulled: the new inner closes a
/// same-instant cycle.
struct Cycle;

/// RFD 5 with its fallback: the mark and flat loop, the two-way small-side
/// check at commit, and memoized pull for the two dynamic cases. The nodes
/// a closure built are pulled once it returns, each after what it depends
/// on; a `switch_cell` switching now pulls its new inner when the loop
/// reaches it. A pull evaluates marked nodes ahead of the loop, so the loop
/// checks a memo stamp before each node.
#[derive(Clone)]
pub struct MarkPull {
    pub run: Run<TwoWay>,
    pub slots: Slots,
    mark: Vec<u32>,
    done: Vec<u32>,
    busy: Vec<u32>,
    order: Vec<Id>,
    stack: Vec<(Id, u32)>,
    /// The first node built this transaction.
    first: Id,
    pub stats: InstantStats,
}

impl MarkPull {
    pub fn new(f: &Fixture) -> Self {
        let n = f.graph.len();
        MarkPull {
            run: Run {
                graph: f.graph.clone(),
                checker: TwoWay::new(&f.graph),
            },
            slots: Slots::new(n, f.pass),
            mark: vec![0; n],
            done: vec![0; n],
            busy: vec![0; n],
            order: Vec::new(),
            stack: Vec::new(),
            first: 0,
            stats: InstantStats::default(),
        }
    }

    fn resize(&mut self) {
        let n = self.run.graph.len();
        self.mark.resize(n, 0);
        self.done.resize(n, 0);
        self.busy.resize(n, 0);
    }

    /// RFD 5's depth-first mark from one more root; the reverse of the
    /// concatenated post-orders is a topological order of the union.
    fn mark_from(&mut self, root: Id) {
        let g = &self.run.graph;
        let tx = self.slots.tx;
        if self.mark[root as usize] == tx {
            return;
        }
        self.mark[root as usize] = tx;
        self.stack.push((root, 0));
        while let Some(top) = self.stack.last_mut() {
            let (n, k) = *top;
            if let Some(&d) = g.dependents[n as usize].get(k as usize) {
                top.1 += 1;
                if self.mark[d as usize] != tx {
                    self.mark[d as usize] = tx;
                    self.stack.push((d, 0));
                }
            } else {
                self.order.push(n);
                self.stack.pop();
            }
        }
    }

    /// Whether `d` still has to be evaluated this transaction: marked or
    /// built, and not yet done. Anything else is quiet.
    #[inline]
    fn pending(&self, d: Id) -> bool {
        let i = d as usize;
        self.done[i] != self.slots.tx && (self.mark[i] == self.slots.tx || d >= self.first)
    }

    fn pull(&mut self, n: Id) -> Result<(), Cycle> {
        let tx = self.slots.tx;
        let i = n as usize;
        if self.busy[i] == tx {
            return Err(Cycle);
        }
        self.busy[i] = tx;
        if self.slots.is_switching(n) {
            let d = self.slots.to[i];
            if self.pending(d) {
                self.pull(d)?;
            }
        } else {
            for k in 0..self.run.graph.deps[i].len() {
                let d = self.run.graph.deps[i][k];
                if self.pending(d) {
                    self.pull(d)?;
                }
            }
        }
        self.slots.eval(&self.run.graph, n);
        self.done[i] = tx;
        if n >= self.first {
            self.stats.pulled += 1;
        } else {
            self.stats.pulled_early += 1;
        }
        Ok(())
    }

    /// The loop's step: a `switch_cell` switching now pulls its new inner
    /// first, and is busy while it does, so a pull that comes back to it
    /// has gone round a cycle.
    #[inline]
    fn step(&mut self, n: Id) -> Result<(), Cycle> {
        if self.slots.is_switching(n) {
            self.busy[n as usize] = self.slots.tx;
            let d = self.slots.to[n as usize];
            if self.pending(d) {
                self.pull(d)?;
            }
        }
        self.slots.eval(&self.run.graph, n);
        self.done[n as usize] = self.slots.tx;
        Ok(())
    }

    /// The closure runs: its subgraphs are built and linked, and the
    /// switches learn where they move.
    fn announce(&mut self, t: &Tx) {
        for b in &t.builds {
            self.run.build(b);
        }
        let len = self.run.graph.len();
        self.slots.grow(len);
        self.resize();
        self.stats.built += (len - self.first as usize) as u64;
        for m in t.moves.iter().filter(|m| is_cell(m)) {
            // Marked from the construct point, or built just now: the
            // generator may move a local switch in the instant it was built.
            debug_assert!(self.pending(m.switch), "{m:?}");
            self.slots.switch_to(m.switch, m.to);
        }
    }

    fn pull_built(&mut self) -> Result<(), Cycle> {
        for n in self.first..self.run.graph.len() as Id {
            if self.done[n as usize] != self.slots.tx {
                self.pull(n)?;
            }
        }
        Ok(())
    }

    /// One transaction: the mark from the input and the construct point,
    /// the loop, the closure and its pulls where the point is evaluated,
    /// then the commit, whose values stand only if it is accepted.
    pub fn tx(&mut self, t: &Tx, (input, value): (Id, u64)) -> Outcome {
        self.slots.begin(input, value);
        self.first = self.run.graph.len() as Id;
        self.order.clear();
        let at = construct_point(t);
        self.mark_from(input);
        match at {
            Some(point) => self.mark_from(point),
            None => {
                for m in t.moves.iter().filter(|m| is_cell(m)) {
                    self.mark_from(m.switch);
                }
            }
        }
        self.stats.visited += self.order.len() as u64;
        let mut poisoned = false;
        if at.is_none() {
            self.announce(t);
            poisoned = self.pull_built().is_err();
        }
        let order = std::mem::take(&mut self.order);
        for &n in order.iter().rev() {
            if poisoned {
                break;
            }
            if self.done[n as usize] == self.slots.tx {
                continue;
            }
            poisoned = self.step(n).is_err();
            if !poisoned && Some(n) == at {
                self.announce(t);
                poisoned = self.pull_built().is_err();
            }
        }
        self.order = order;
        self.stats.poisoned += poisoned as u64;
        let accepted = self.run.checker.commit(&mut self.run.graph, &t.moves) && !poisoned;
        self.slots.finish(accepted)
    }

    /// Every transaction with its event; the sum of the digests.
    pub fn all(&mut self, txs: &[Tx], events: &[(Id, u64)]) -> u64 {
        txs.iter()
            .zip(events)
            .fold(0, |acc, (t, &e)| acc.wrapping_add(self.tx(t, e).digest))
    }
}

/// [`MarkPull`] without the construct point forced into its mark, to see
/// how much of the heights' lead is the region that forcing adds.
///
/// Forcing the root event stream's forward token into every navigation's
/// mark marks everything downstream of it, which is every view. Here the
/// mark starts from the fired input alone, plus each existing
/// `switch_cell` that moves: a switch has no selector in the graph, and at
/// a switch instant it emits whatever its new inner did, so it stands in
/// for the selector firing and marking it. The closure runs at the start,
/// as for F46's pair, since the selector that learns the moves is off the
/// graph too; the nodes it built are pulled once it returns, each after
/// what it depends on, and a switch built and moved in the same instant is
/// pulled with them. A moving `switch_cell` pulls its new inner when the
/// loop reaches it, and every move, `switch_stream`s' too, is checked at
/// commit, as in [`MarkPull`].
#[derive(Clone)]
pub struct MarkUnforced(pub MarkPull);

impl MarkUnforced {
    pub fn new(f: &Fixture) -> Self {
        MarkUnforced(MarkPull::new(f))
    }

    /// One transaction: the mark from the input and the moving switches,
    /// the closure and its pulls, the loop, then the commit, whose values
    /// stand only if it is accepted.
    pub fn tx(&mut self, t: &Tx, (input, value): (Id, u64)) -> Outcome {
        let e = &mut self.0;
        e.slots.begin(input, value);
        e.first = e.run.graph.len() as Id;
        e.order.clear();
        e.mark_from(input);
        for m in t.moves.iter().filter(|m| is_cell(m)) {
            // One built this instant doesn't exist yet; it's pulled.
            if m.switch < e.first {
                e.mark_from(m.switch);
            }
        }
        e.stats.visited += e.order.len() as u64;
        e.announce(t);
        let mut poisoned = e.pull_built().is_err();
        let order = std::mem::take(&mut e.order);
        for &n in order.iter().rev() {
            if poisoned {
                break;
            }
            if e.done[n as usize] == e.slots.tx {
                continue;
            }
            poisoned = e.step(n).is_err();
        }
        e.order = order;
        e.stats.poisoned += poisoned as u64;
        let accepted = e.run.checker.commit(&mut e.run.graph, &t.moves) && !poisoned;
        e.slots.finish(accepted)
    }

    /// Every transaction with its event; the sum of the digests.
    pub fn all(&mut self, txs: &[Tx], events: &[(Id, u64)]) -> u64 {
        txs.iter()
            .zip(events)
            .fold(0, |acc, (t, &e)| acc.wrapping_add(self.tx(t, e).digest))
    }

    pub fn stats(&self) -> &InstantStats {
        &self.0.stats
    }

    pub fn values(&self) -> &[u64] {
        self.0.slots.values()
    }
}

/// Pushes `n` into the bucket of its height now.
#[inline]
fn push(buckets: &mut Vec<Vec<Id>>, height: &[u32], n: Id, top: &mut usize) {
    let h = height[n as usize] as usize;
    if buckets.len() <= h {
        buckets.resize(h + 1, Vec::new());
    }
    buckets[h].push(n);
    *top = (*top).max(h);
}

/// Pushes `n`'s dependents above `floor` not queued yet this transaction.
#[inline]
#[allow(clippy::too_many_arguments)]
fn push_dependents(
    g: &Graph,
    height: &[u32],
    buckets: &mut Vec<Vec<Id>>,
    queued: &mut [u32],
    tx: u32,
    n: Id,
    floor: u32,
    top: &mut usize,
) {
    for &d in &g.dependents[n as usize] {
        if queued[d as usize] != tx && height[d as usize] > floor {
            queued[d as usize] = tx;
            push(buckets, height, d, top);
        }
    }
}

/// Heights and a bucket queue, with the two dynamic cases. When the
/// closure runs, its nodes get heights as they are built, and each with a
/// fired dependency is queued; one at or below the cursor either sends the
/// cursor back down to it (`RESEAT`) or is evaluated at once, in creation
/// order, out of the height order (`!RESEAT`, what RFD 5's pull does here,
/// since creation order is a topological order of what a closure builds).
/// Then the `switch_cell`s that move are relinked to their new inners,
/// each raised above its inner if it isn't; a raise that comes back to the
/// inner is a cycle, and poisons. A queued node a raise moved up is found
/// at its old height and pushed again at its new one. The
/// `switch_stream`s move at commit.
#[derive(Clone)]
pub struct HeightsAt<const RESEAT: bool> {
    pub run: Run<Heights>,
    pub slots: Slots,
    buckets: Vec<Vec<Id>>,
    queued: Vec<u32>,
    cells: Vec<Move>,
    streams: Vec<Move>,
    pub stats: InstantStats,
}

/// Heights re-seating the cursor for built nodes below it.
pub type HeightsReseat = HeightsAt<true>;
/// Heights evaluating built nodes below the cursor at once.
pub type HeightsPull = HeightsAt<false>;

impl<const RESEAT: bool> HeightsAt<RESEAT> {
    pub fn new(f: &Fixture) -> Self {
        let checker = <Heights as Checker>::new(&f.graph);
        HeightsAt {
            buckets: vec![Vec::new(); checker.max as usize + 1],
            queued: vec![0; f.graph.len()],
            run: Run {
                graph: f.graph.clone(),
                checker,
            },
            slots: Slots::new(f.graph.len(), f.pass),
            cells: Vec::new(),
            streams: Vec::new(),
            stats: InstantStats::default(),
        }
    }

    /// The closure runs with the cursor at `cursor`; `h` is the cursor to
    /// go on from. Err on a cycle. Kept out of line: inlined, it moved the
    /// two variants 10% apart on `settled`, where they do the same work.
    #[inline(never)]
    fn announce(
        &mut self,
        t: &Tx,
        cursor: usize,
        h: &mut usize,
        top: &mut usize,
    ) -> Result<(), Cycle> {
        let tx = self.slots.tx;
        let first = self.run.graph.len();
        for b in &t.builds {
            self.run.build(b);
        }
        let len = self.run.graph.len();
        self.slots.grow(len);
        self.queued.resize(len, 0);
        self.stats.built += (len - first) as u64;
        let g = &self.run.graph;
        let height = &self.run.checker.height;
        for n in first as Id..len as Id {
            let hn = height[n as usize] as usize;
            let below = hn <= cursor;
            self.stats.below += below as u64;
            if self.queued[n as usize] == tx
                || !g.deps[n as usize].iter().any(|&d| self.slots.has_fired(d))
            {
                // Queued by a node evaluated out of order just now; or
                // quiet, or a dependency above the cursor queues it when it
                // fires.
                continue;
            }
            self.queued[n as usize] = tx;
            if !below {
                push(&mut self.buckets, height, n, top);
            } else if RESEAT {
                push(&mut self.buckets, height, n, top);
                if hn < *h {
                    *h = hn;
                    self.stats.reseats += 1;
                }
            } else {
                // Its dependencies are all below the cursor and done, or
                // built before it. Those of its dependents at or below the
                // cursor were built after it and come to this loop.
                self.stats.out_of_order += 1;
                if self.slots.eval(g, n) {
                    let (b, q) = (&mut self.buckets, &mut self.queued);
                    push_dependents(g, height, b, q, tx, n, cursor as u32, top);
                }
            }
        }

        self.cells.clear();
        self.cells
            .extend(t.moves.iter().filter(|m| is_cell(m)).copied());
        let s = &self.run.checker.stats;
        let (raises, touched) = (s.raises, s.touched);
        let ok = self.run.checker.commit(&mut self.run.graph, &self.cells);
        let s = &self.run.checker.stats;
        self.stats.raises += s.raises - raises;
        self.stats.touched += s.touched - touched;
        if !ok {
            return Err(Cycle);
        }
        let height = &self.run.checker.height;
        for m in &self.cells {
            self.slots.switch_to(m.switch, m.to);
            let s = m.switch as usize;
            debug_assert!(
                height[s] as usize > cursor,
                "a switch at or below the cursor"
            );
            if self.queued[s] != tx {
                self.queued[s] = tx;
                push(&mut self.buckets, height, m.switch, top);
            }
        }
        Ok(())
    }

    /// One transaction: the input's dependents and the construct point
    /// queued, the climb, the closure and the raises where the point is
    /// evaluated, then the `switch_stream`s' commit, whose values stand
    /// only if it is accepted.
    pub fn tx(&mut self, t: &Tx, (input, value): (Id, u64)) -> Outcome {
        self.slots.begin(input, value);
        let tx = self.slots.tx;
        self.queued.resize(self.run.graph.len(), 0);
        let at = construct_point(t);
        let start = self.run.checker.height[input as usize];
        let mut h = start as usize + 1;
        let mut top = 0;
        {
            let (g, height) = (&self.run.graph, &self.run.checker.height);
            let (b, q) = (&mut self.buckets, &mut self.queued);
            push_dependents(g, height, b, q, tx, input, start, &mut top);
        }
        let mut poisoned = false;
        match at {
            Some(point) => {
                if self.queued[point as usize] != tx {
                    self.queued[point as usize] = tx;
                    push(&mut self.buckets, &self.run.checker.height, point, &mut top);
                }
            }
            None => poisoned = self.announce(t, start as usize, &mut h, &mut top).is_err(),
        }
        while !poisoned && h <= top {
            let Some(n) = self.buckets[h].pop() else {
                h += 1;
                self.stats.stepped += 1;
                continue;
            };
            let height = &self.run.checker.height;
            if height[n as usize] as usize != h {
                self.stats.stale += 1;
                push(&mut self.buckets, height, n, &mut top);
                continue;
            }
            self.stats.visited += 1;
            if self.slots.eval(&self.run.graph, n) {
                let (g, b, q) = (&self.run.graph, &mut self.buckets, &mut self.queued);
                push_dependents(g, height, b, q, tx, n, h as u32, &mut top);
            }
            if Some(n) == at {
                let cursor = h;
                poisoned = self.announce(t, cursor, &mut h, &mut top).is_err();
            }
        }
        if poisoned {
            self.stats.poisoned += 1;
            for b in self.buckets.iter_mut().take(top + 1) {
                b.clear();
            }
        }
        let accepted = !poisoned && {
            self.streams.clear();
            self.streams
                .extend(t.moves.iter().filter(|m| !is_cell(m)).copied());
            let ok = self.run.checker.commit(&mut self.run.graph, &self.streams);
            if !ok {
                // Put the switch_cells back where the instant found them.
                let back: Vec<Move> = self
                    .cells
                    .iter()
                    .map(|m| Move {
                        from: m.to,
                        to: m.from,
                        ..*m
                    })
                    .collect();
                assert!(self.run.checker.commit(&mut self.run.graph, &back));
            }
            ok
        };
        self.slots.finish(accepted)
    }

    /// Every transaction with its event; the sum of the digests.
    pub fn all(&mut self, txs: &[Tx], events: &[(Id, u64)]) -> u64 {
        txs.iter()
            .zip(events)
            .fold(0, |acc, (t, &e)| acc.wrapping_add(self.tx(t, e).digest))
    }
}

/// The reference for the instant engines: everything built first, the
/// `switch_cell`s relinked, and every node evaluated in a topological
/// order of the whole graph, found afresh by Kahn's algorithm; the walk at
/// commit. It asserts that the instant's graph has a cycle exactly when the
/// walk refuses the final one: here they differ only in the
/// `switch_stream`s' inners, and nothing downstream of a view is an event.
#[derive(Clone)]
pub struct Recompute {
    pub run: Run<Baseline>,
    pub slots: Slots,
    indeg: Vec<u32>,
    order: Vec<Id>,
}

impl Recompute {
    pub fn new(f: &Fixture) -> Self {
        Recompute {
            run: Run {
                graph: f.graph.clone(),
                checker: Baseline::new(&f.graph),
            },
            slots: Slots::new(f.graph.len(), f.pass),
            indeg: Vec::new(),
            order: Vec::new(),
        }
    }

    pub fn tx(&mut self, t: &Tx, (input, value): (Id, u64)) -> Outcome {
        self.slots.begin(input, value);
        for b in &t.builds {
            self.run.build(b);
        }
        let g = &mut self.run.graph;
        self.slots.grow(g.len());
        let cells: Vec<Move> = t.moves.iter().filter(|m| is_cell(m)).copied().collect();
        for m in &cells {
            g.unlink(m.from, m.switch);
        }
        for m in &cells {
            g.link(m.to, m.switch);
            self.slots.switch_to(m.switch, m.to);
        }
        self.indeg.clear();
        self.indeg.extend(g.deps.iter().map(|d| d.len() as u32));
        self.order.clear();
        self.order
            .extend((0..g.len() as Id).filter(|&n| g.deps[n as usize].is_empty()));
        let mut k = 0;
        while k < self.order.len() {
            let n = self.order[k];
            k += 1;
            for &d in &g.dependents[n as usize] {
                self.indeg[d as usize] -= 1;
                if self.indeg[d as usize] == 0 {
                    self.order.push(d);
                }
            }
        }
        let acyclic = self.order.len() == g.len();
        if acyclic {
            for &n in &self.order {
                self.slots.eval(g, n);
            }
        }
        for m in &cells {
            g.unlink(m.to, m.switch);
        }
        for m in &cells {
            g.link(m.from, m.switch);
        }
        let accepted = self.run.checker.commit(&mut self.run.graph, &t.moves);
        assert_eq!(
            acyclic, accepted,
            "the instant's cycles are the final graph's"
        );
        self.slots.finish(accepted)
    }
}

/// Runs the instant engines over the fixture against [`Recompute`], and
/// panics unless each transaction fired the same nodes with the same
/// values and got the same verdict, the heights hold every edge after each
/// (in debug builds), and the committed values end the same.
pub fn agree_instant(f: &Fixture) {
    let mut reference = Recompute::new(f);
    let mut mark = MarkPull::new(f);
    let mut reseat = HeightsReseat::new(f);
    let mut pull = HeightsPull::new(f);
    let mut unforced = MarkUnforced::new(f);
    for (k, (t, &e)) in f.txs.iter().zip(&f.events).enumerate() {
        let o = reference.tx(t, e);
        let others = [
            mark.tx(t, e),
            reseat.tx(t, e),
            pull.tx(t, e),
            unforced.tx(t, e),
        ];
        assert!(others.iter().all(|x| *x == o), "tx {k}: {o:?} {others:?}");
        debug_assert!(reseat.run.checker.valid(&reseat.run.graph), "tx {k}");
        debug_assert!(pull.run.checker.valid(&pull.run.graph), "tx {k}");
    }
    let v = reference.slots.values();
    assert!(mark.slots.values() == v && reseat.slots.values() == v && pull.slots.values() == v);
    assert!(unforced.values() == v);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rfd_0005_bounded_relink_check::{
        Baseline, KINDS, Kind, Params, Workload, generate, workloads,
    };
    use crate::rfd_0005_heap_vs_mark_on_quiet_regions::Rng;
    use crate::rfd_0005_small_side_order::TwoWay;

    /// The fixture's graph and transactions, for a checker alone.
    fn workload_of(f: &Fixture) -> Workload {
        Workload {
            graph: f.graph.clone(),
            txs: f.txs.clone(),
            refused: Vec::new(),
        }
    }

    #[test]
    fn engines_agree() {
        for name in WORKLOADS {
            for pass in [100, 20, 0] {
                agree(&Fixture::new(name, pass));
            }
        }
    }

    #[test]
    fn instant_engines_agree() {
        for name in WORKLOADS {
            for pass in [100, 20, 0] {
                agree_instant(&Fixture::new(name, pass));
            }
        }
    }

    /// The instant engines per transaction, per workload and pass rate.
    /// Run with `--nocapture`.
    #[test]
    fn counts_instant() {
        println!(
            "{:<8} {:>4} {:>7} {:>7} {:>6} {:>6} {:>6} {:>6} {:>7} {:>6} {:>6} {:>6} {:>6} {:>6} {:>6} {:>6} {:>6}",
            "workload",
            "pass",
            "marked",
            "fired",
            "quiet%",
            "built",
            "pulled",
            "early",
            "popped",
            "below",
            "ooo",
            "reseat",
            "step-r",
            "step-p",
            "raises",
            "touch",
            "stale",
        );
        let mut txs = 0;
        for name in WORKLOADS {
            for pass in PASS {
                let f = Fixture::new(name, pass);
                agree_instant(&f);
                txs = f.txs.len();
                let mut mark = MarkPull::new(&f);
                let mut reseat = HeightsReseat::new(&f);
                let mut pull = HeightsPull::new(&f);
                let mut fired = 0u64;
                for (t, &e) in f.txs.iter().zip(&f.events) {
                    fired += mark.tx(t, e).fired as u64;
                    reseat.tx(t, e);
                    pull.tx(t, e);
                }
                let (m, r, p) = (&mark.stats, &reseat.stats, &pull.stats);
                assert_eq!(r.visited, p.visited + p.out_of_order);
                assert_eq!((r.raises, r.touched), (p.raises, p.touched));
                assert_eq!(m.poisoned, r.poisoned);
                let n = f.txs.len() as f64;
                let per = |x: u64| x as f64 / n;
                println!(
                    "{:<8} {:>4} {:>7.1} {:>7.1} {:>6.1} {:>6.1} {:>6.1} {:>6.1} {:>7.1} {:>6.1} {:>6.1} {:>6.2} {:>6.1} {:>6.1} {:>6.2} {:>6.1} {:>6.2}",
                    name,
                    pass,
                    per(m.visited),
                    per(fired),
                    100.0 * (1.0 - fired as f64 / m.visited as f64),
                    per(m.built),
                    per(m.pulled),
                    per(m.pulled_early),
                    per(p.visited),
                    per(p.below),
                    per(p.out_of_order),
                    per(r.reseats),
                    per(r.stepped),
                    per(p.stepped),
                    per(p.raises),
                    per(p.touched),
                    per(p.stale),
                );
            }
        }
        println!();
        println!("per transaction, over each workload's {txs} transactions, with nodes built");
        println!("during the instant evaluated in it and each switch_cell reading its new");
        println!("inner at the switch instant. Mark and pull: nodes marked, fired (refused");
        println!("transactions count none), the quiet share of the marked region, nodes");
        println!("built, built nodes pulled once the closure returned, and existing nodes");
        println!("pulled ahead of the flat loop. Heights: nodes popped by the pulling");
        println!("variant (the re-seating one pops these plus ooo), built nodes at or below");
        println!("the cursor when built, those of them evaluated out of order (ooo) by the");
        println!("pulling variant, times the re-seating variant sent its cursor back down,");
        println!("empty buckets stepped over by each (r, p), switch_cell links made");
        println!("mid-evaluation that needed a raise, nodes those raises touched, and queued");
        println!("nodes a raise moved and the cursor found at their old height. pass is the");
        println!("filters' pass rate in percent.");

        println!();
        println!("{:<8} {:>6} {:>6}", "workload", "refus", "poison");
        for name in WORKLOADS {
            let f = Fixture::new(name, 20);
            let mut mark = MarkPull::new(&f);
            let mut refused = 0;
            for (t, &e) in f.txs.iter().zip(&f.events) {
                refused += !mark.tx(t, e).accepted as u32;
            }
            println!("{:<8} {:>6} {:>6}", name, refused, mark.stats.poisoned);
        }
        println!();
        println!("transactions refused, and of them found to close a cycle during");
        println!("evaluation (by a pull coming back to the switch_cell, or by a raise");
        println!("coming back to the new inner; the two engines agree), whatever the pass");
        println!("rate.");
    }

    /// The mark's region with and without the construct point forced into
    /// it, beside what fires and what the heights pop. Run with
    /// `--nocapture`.
    #[test]
    fn counts_unforced() {
        println!(
            "{:<8} {:>4} {:>7} {:>7} {:>7} {:>6} {:>6} {:>6} {:>6} {:>7}",
            "workload",
            "pass",
            "forced",
            "unforc",
            "fired",
            "q-forc",
            "q-unf",
            "e-forc",
            "e-unf",
            "popped",
        );
        let mut txs = 0;
        for name in WORKLOADS {
            for pass in PASS {
                let f = Fixture::new(name, pass);
                agree_instant(&f);
                txs = f.txs.len();
                let mut forced = MarkPull::new(&f);
                let mut unforced = MarkUnforced::new(&f);
                let mut pull = HeightsPull::new(&f);
                let mut fired = 0u64;
                for (t, &e) in f.txs.iter().zip(&f.events) {
                    fired += forced.tx(t, e).fired as u64;
                    unforced.tx(t, e);
                    pull.tx(t, e);
                }
                let (m, u, p) = (&forced.stats, unforced.stats(), &pull.stats);
                assert_eq!((m.built, m.pulled), (u.built, u.pulled));
                assert_eq!(m.poisoned, u.poisoned);
                let n = f.txs.len() as f64;
                let per = |x: u64| x as f64 / n;
                let quiet = |marked: u64| 100.0 * (1.0 - fired as f64 / marked as f64);
                println!(
                    "{:<8} {:>4} {:>7.1} {:>7.1} {:>7.1} {:>6.1} {:>6.1} {:>6.1} {:>6.1} {:>7.1}",
                    name,
                    pass,
                    per(m.visited),
                    per(u.visited),
                    per(fired),
                    quiet(m.visited),
                    quiet(u.visited),
                    per(m.pulled_early),
                    per(u.pulled_early),
                    per(p.visited + p.out_of_order),
                );
            }
        }
        println!();
        println!("per transaction, over each workload's {txs} transactions, with nodes built");
        println!("during the instant evaluated in it: nodes marked with the construct point");
        println!("(the root event stream's forward token) forced into the mark, and without");
        println!("it (from the input and the moving switch_cells only); nodes fired (refused");
        println!("transactions count none); the quiet share of each marked region; existing");
        println!("nodes each pulled ahead of the flat loop; and nodes the heights evaluated");
        println!("(popped, plus built nodes evaluated out of order). Both marks pull the same");
        println!("built nodes. pass is the filters' pass rate in percent.");
    }

    /// Through the checker alone: the heights refuse exactly what the walk
    /// refuses, F46's reversal included, and hold every edge after every
    /// transaction.
    #[test]
    fn heights_refuse_what_the_walk_refuses() {
        for name in WORKLOADS {
            let w = workload_of(&Fixture::new(name, 0));
            let mut walk = Run::<Baseline>::new(&w);
            let mut h = Run::<Heights>::new(&w);
            for t in &w.txs {
                assert_eq!(walk.tx(t), h.tx(t));
                assert!(h.checker.valid(&h.graph));
            }
            assert!(h.checker.stats.refused > 0, "{name} has no cycle");
        }
    }

    fn per_kind(c: &Counts, k: Kind) -> (u64, u64, u64) {
        let i = k as usize;
        (c.moves[i], c.invalid[i], c.visited[i])
    }

    /// Evaluation per transaction through each scheduler, per workload and
    /// pass rate; then the upkeep, whatever the pass rate. Run with
    /// `--nocapture`.
    #[test]
    fn counts() {
        println!(
            "{:<8} {:>4} {:>8} {:>8} {:>7} {:>8} {:>8} {:>8}",
            "workload", "pass", "marked", "fired", "quiet%", "popped", "stepped", "labelled"
        );
        let mut txs = 0;
        for name in WORKLOADS {
            for pass in PASS {
                let f = Fixture::new(name, pass);
                agree(&f);
                txs = f.txs.len();
                let mut mark = MarkOm::new(&f);
                let mut heights = HeightQueue::new(&f);
                let mut radix = RadixQueue::new(&f);
                let (mut marked, mut fired, mut popped, mut stepped, mut labelled) =
                    (0u64, 0u64, 0u64, 0u64, 0u64);
                for (t, &e) in f.txs.iter().zip(&f.events) {
                    fired += mark.tx(t, e).fired as u64;
                    marked += mark.visited() as u64;
                    heights.tx(t, e);
                    popped += heights.visited() as u64;
                    stepped += heights.sched.stepped as u64;
                    radix.tx(t, e);
                    labelled += radix.visited() as u64;
                }
                let n = f.txs.len() as f64;
                println!(
                    "{:<8} {:>4} {:>8.1} {:>8.1} {:>7.1} {:>8.1} {:>8.1} {:>8.1}",
                    name,
                    pass,
                    marked as f64 / n,
                    fired as f64 / n,
                    100.0 * (1.0 - fired as f64 / marked as f64),
                    popped as f64 / n,
                    stepped as f64 / n,
                    labelled as f64 / n,
                );
            }
        }
        println!();
        println!("per transaction, over each workload's {txs} transactions: nodes marked by the");
        println!("depth-first mark, fired, the quiet share of the marked region, popped by the");
        println!("height buckets, empty buckets their cursor stepped over, and popped by the");
        println!("label-ordered queue of the maintained-rank probe. pass is the filters'");
        println!("pass rate in percent.");
        println!();

        println!(
            "{:<8} {:>5} {:>5} {:>6} {:>7} {:>7} {:>8} {:>5} {:>7} {:>8} {:>4} {:>4}",
            "workload",
            "moves",
            "built",
            "raises",
            "touched",
            "largest",
            "restored",
            "refus",
            "two-way",
            "walk",
            "h0",
            "h300",
        );
        for name in WORKLOADS {
            let w = workload_of(&Fixture::new(name, 0));
            let mut walk = Run::<Baseline>::new(&w);
            let mut om = Run::<TwoWay>::new(&w);
            let mut h = Run::<Heights>::new(&w);
            let h0 = h.checker.max;
            walk.all(&w.txs);
            om.all(&w.txs);
            h.all(&w.txs);
            let s = &h.checker.stats;
            let sum = |c: &Counts| c.visited.iter().sum::<u64>();
            println!(
                "{:<8} {:>5} {:>5} {:>6} {:>7} {:>7} {:>8} {:>5} {:>7} {:>8} {:>4} {:>4}",
                name,
                w.moves(),
                s.built,
                s.raises,
                s.touched,
                s.largest,
                s.restored,
                s.refused,
                sum(om.checker.counts()),
                sum(walk.checker.counts()),
                h0,
                h.checker.max,
            );
        }
        println!();
        println!("totals over each workload's {txs} transactions, whatever the pass rate:");
        println!("moves, nodes built and given a height, links that needed a raise, nodes");
        println!("those raises touched (took out of the adjust heap), the most one link");
        println!("touched, nodes touched after a refusal finishing the raises and relinking,");
        println!("transactions refused; then nodes the two-way small-side check and the");
        println!("upstream walk visited for the same moves; and the largest height before");
        println!("the first transaction and after the last.");
        println!();

        println!(
            "{:<8} {:<6} {:>6} {:>7} {:>8} {:>8} {:>8}",
            "workload", "kind", "moves", "raises", "touched", "two-way", "walk"
        );
        for name in WORKLOADS {
            let w = workload_of(&Fixture::new(name, 0));
            let mut walk = Run::<Baseline>::new(&w);
            let mut om = Run::<TwoWay>::new(&w);
            let mut h = Run::<Heights>::new(&w);
            walk.all(&w.txs);
            om.all(&w.txs);
            h.all(&w.txs);
            for k in KINDS {
                let (moves, raises, touched) = per_kind(h.checker.counts(), k);
                let (_, _, two) = per_kind(om.checker.counts(), k);
                let (_, _, walked) = per_kind(walk.checker.counts(), k);
                println!(
                    "{:<8} {:<6} {:>6} {:>7} {:>8} {:>8} {:>8}",
                    name,
                    format!("{k:?}").to_lowercase(),
                    moves,
                    raises,
                    touched,
                    two,
                    walked
                );
            }
        }
        println!();
        println!("the same by kind of move: a switch_cell over screen views, a switch_stream");
        println!("over screen events, a local switch, a view move retargeted to close a");
        println!("cycle, and F46's pair. Refusals' restoring is not in these.");
    }

    /// A workload of `txs` transactions, with an event for each, and
    /// `p_bad` of them trying to close a cycle.
    fn long(name: &str, txs: usize, pass: u32, p_bad: f64) -> Fixture {
        let (_, p) = workloads(txs)
            .into_iter()
            .find(|(n, _)| *n == name)
            .expect("no such workload");
        let w = generate(&Params { p_bad, ..p });
        let mut rng = Rng::new(0x5eed_5005);
        let events = w
            .txs
            .iter()
            .map(|_| (rng.below(64), rng.next_u64()))
            .collect();
        Fixture {
            graph: w.graph,
            txs: w.txs,
            events,
            pass,
        }
    }

    /// Height growth over ten times the workloads' length. Run with
    /// `--nocapture`.
    #[test]
    fn growth() {
        const LONG: usize = 3000;
        const EVERY: usize = 300;
        println!(
            "{:<8} {:>5} {:>6} {:>5} {:>6} {:>6} {:>7} {:>7} {:>7}",
            "workload", "tx", "nodes", "max", "mean", "raises", "touched", "stepped", "popped"
        );
        for name in WORKLOADS {
            let f = long(name, LONG, 20, workloads(LONG)[0].1.p_bad);
            let mut e = HeightQueue::new(&f);
            let mut mark = MarkOm::new(&f);
            let (mut raises, mut touched) = (0, 0);
            let (mut stepped, mut popped) = (0u64, 0u64);
            let mean = |hs: &[u32]| hs.iter().map(|&h| h as f64).sum::<f64>() / hs.len() as f64;
            for (k, (t, &ev)) in f.txs.iter().zip(&f.events).enumerate() {
                let o = e.tx(t, ev);
                assert_eq!(o, mark.tx(t, ev), "tx {k}");
                stepped += e.sched.stepped as u64;
                popped += e.visited() as u64;
                if (k + 1) % EVERY == 0 {
                    let c = &e.run.checker;
                    let s = &c.stats;
                    println!(
                        "{:<8} {:>5} {:>6} {:>5} {:>6.1} {:>6} {:>7} {:>7.1} {:>7.1}",
                        name,
                        k + 1,
                        c.height.len(),
                        c.max,
                        mean(&c.height),
                        s.raises - raises,
                        s.touched + s.restored - touched,
                        stepped as f64 / EVERY as f64,
                        popped as f64 / EVERY as f64,
                    );
                    raises = s.raises;
                    touched = s.touched + s.restored;
                    stepped = 0;
                    popped = 0;
                }
            }
            let tight = Heights::<true>::from_scratch(&e.run.graph);
            println!(
                "{:<8} {:>5} {:>6} {:>5} {:>6.1}   longest paths of the final graph",
                name,
                "",
                tight.len(),
                tight.iter().max().unwrap(),
                mean(&tight),
            );
        }
        println!();
        println!("every {EVERY} transactions of a {LONG}-transaction run at pass 20%: the");
        println!("nodes, the largest height any node has had and the mean height now, the");
        println!("raises and the nodes they touched in the window (refusals' restoring");
        println!("included), and per transaction the empty buckets the evaluation cursor");
        println!("stepped over and the nodes it popped. The last line of each workload is");
        println!("the longest-path heights of its final graph, the least they could be.");
        println!();

        println!(
            "{:<8} {:>6} {:>6} {:>6} {:>6} {:>6}",
            "workload", "h0", "h1000", "h2000", "h3000", "tight"
        );
        for name in WORKLOADS {
            let f = long(name, LONG, 0, 0.0);
            let w = workload_of(&f);
            let mut h = Run::<Heights>::new(&w);
            let mut at = vec![h.checker.max];
            for (k, t) in w.txs.iter().enumerate() {
                assert!(h.tx(t), "tx {k} refused with no cycle tried");
                if (k + 1) % 1000 == 0 {
                    at.push(h.checker.max);
                }
            }
            let tight = Heights::<true>::from_scratch(&h.graph);
            println!(
                "{:<8} {:>6} {:>6} {:>6} {:>6} {:>6}",
                name,
                at[0],
                at[1],
                at[2],
                at[3],
                tight.iter().max().unwrap()
            );
        }
        println!();
        println!("the same runs with no transaction trying to close a cycle: the largest");
        println!("height at the start and every 1000 transactions, and the longest path of");
        println!("the final graph.");
    }

    /// The cap as the only cycle detector: what the raises cost before the
    /// cap stops them, and whether it refuses exactly what the walk does.
    /// Run with `--nocapture`.
    #[test]
    fn cap_alone() {
        println!(
            "{:<8} {:>5} {:>6} {:>6} {:>6} {:>9} {:>9}",
            "workload", "cap", "walk", "capped", "false", "guard-tch", "cap-tch"
        );
        for name in WORKLOADS {
            let w = workload_of(&Fixture::new(name, 0));
            let h0 = Run::<Heights>::new(&w).checker.max;
            for cap in [128, 2 * h0, 4 * h0] {
                let mut walk = Run::<Baseline>::new(&w);
                let mut g = Run::<Heights>::new(&w);
                let mut c = Run::<Heights<false>>::new(&w);
                c.checker.cap = cap;
                let (mut refused, mut capped, mut wrong) = (0, 0, 0);
                let (mut g_touched, mut c_touched) = (0, 0);
                for t in &w.txs {
                    let before = (g.checker.stats.touched, c.checker.stats.touched);
                    let ok = walk.tx(t);
                    assert_eq!(ok, g.tx(t));
                    let by_cap = !c.tx(t);
                    refused += !ok as u32;
                    capped += by_cap as u32;
                    wrong += (ok && by_cap) as u32;
                    if !ok {
                        g_touched += g.checker.stats.touched - before.0;
                        c_touched += c.checker.stats.touched - before.1;
                    }
                }
                println!(
                    "{:<8} {:>5} {:>6} {:>6} {:>6} {:>9} {:>9}",
                    name, cap, refused, capped, wrong, g_touched, c_touched
                );
            }
        }
        println!();
        println!("per workload of 300 transactions, with the cap at Incremental's default");
        println!("and at two and four times the starting height: transactions the walk");
        println!("refused, refused by the cap alone, refused by it though acyclic, and the");
        println!("nodes the raises touched in the transactions the walk refused, with the");
        println!("original-child check and with the cap alone (restoring not included).");
        println!("After a refusal by the cap, heights are recomputed from scratch.");
    }
}
