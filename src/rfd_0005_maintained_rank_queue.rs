//! Does a queue keyed by a maintained topological order still beat RFD 5's
//! mark plus flat loop once the order has to be kept under switching, with
//! its upkeep counted?
//!
//! RFD 5 rejects rank-ordered push because ranks must be maintained as
//! switches move. Two earlier probes bear on that, each with half of it.
//! `rfd-0005-heap-vs-mark-on-quiet-regions` found a queue that visits only
//! firing nodes beats the mark from about 16% quiet upward, with static
//! ranks. `rfd-0005-small-side-order` found an order-maintenance list,
//! updated by a small-side search at each move, keeps the relink check below
//! the upstream walk, for about 84 instructions a node built. Here the list
//! supplies the queue's keys, on the switching graph and the workloads of
//! `rfd-0005-bounded-relink-check`, with events firing through it, and every
//! transaction pays for everything: evaluation, the list's upkeep at node
//! builds and at moves, and the relink check.
//!
//! A transaction is one input event, evaluated, then the subgraphs the
//! workload builds during it, then its commit: the switches move and the
//! checker accepts or refuses them. Four engines run it, and must agree on
//! every transaction's fired set, values and verdict ([`agree`]):
//!
//! - [`Walked`], the baseline: RFD 5 as written, the depth-first mark and
//!   flat loop, and the spike's upstream walk per move.
//! - [`MarkOm`]: the same mark and loop, with the small-side check (probe
//!   14's two-way list) in place of the walk: the fairer baseline, since
//!   the walk is the part of RFD 5 the queue doesn't need to beat.
//! - [`RadixQueue`]: pops firing nodes in list-label order from a radix
//!   heap over the `u64` labels, pushing a node's dependents only when it
//!   fires. Every key pushed is above the last popped, since a dependent is
//!   after its dependency in the order, so the queue is monotone, and a
//!   radix heap is exact for a monotone queue: a push is O(1), and a key
//!   moves down at most 64 buckets over its life, with no comparison
//!   against the rest of the queue. A bucket per label is out: labels are
//!   sparse `u64`s, up to 2^62.
//! - [`HeapQueue`]: the same with a binary heap, the log factor RFD 5
//!   argued against, for scale.
//!
//! What fires: a node that reads an input or the root event stream (a
//! component's event handler, or a reducer of the application state) is a
//! filter that passes `pass` percent of its events, by hash. Every other
//! node (a lift, a fold, a merge, a switch) fires when any dependency
//! fired. A node's value mixes every dependency's post-instant value, the
//! slot of one that fired and the committed value of one that didn't, so
//! evaluating a node before a dependency changes the digest.
//!
//! Left out, the same for every engine: nodes built during the instant are
//! not evaluated in it (RFD 5 pulls them, as it pulls a `switch_cell`'s new
//! inner at the switch instant), and switches move only at commit, so the
//! order is never repaired mid-transaction. What Sodium re-ranks for
//! mid-transaction is that pull's job here, in both designs.
//!
//! The graph is the relink probe's, a `Vec<Vec<Id>>` per direction. To
//! check that the ratios aren't that layout's, [`FlatEngine`] runs the same
//! schedulers over [`Flat`]: one edge array per direction, each node's list
//! a slice of it, laid out in id order, with the checker still keeping its
//! order over the nested graph and the flat arrays following each build
//! and accepted move. [`Climb`] is the height-queue probe's bucket queue
//! over them, with that probe's [`Heights`] as the checker, and
//! [`agree_flat`] holds the flat engines to the nested walk's results.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::rfd_0005_bounded_relink_check::{Baseline, Checker, Graph, Id, Move, Run, Tx, workload};
use crate::rfd_0005_heap_vs_mark_on_quiet_regions::Rng;
use crate::rfd_0005_height_queue::Heights;
use crate::rfd_0005_small_side_order::TwoWay;

/// The relink probe's generator builds 64 inputs, then the root event
/// stream's forward token, so a node reading any id up to this one reads
/// an input or the root events.
const ENTRY: Id = 64;

/// The inputs events are sent to.
const INPUTS: u32 = 64;

/// The filters' pass rates the sweep runs, in percent.
pub const PASS: [u32; 5] = [100, 50, 30, 20, 5];

/// The workloads, from the relink probe: screens built eagerly
/// (`settled`), sometimes during the instant (`mixed`, `churn`), or always
/// (`lazy`).
pub const WORKLOADS: [&str; 4] = ["settled", "mixed", "churn", "lazy"];

/// A few multiply-xors: node-local work, and the filters' hash.
#[inline]
fn mix(a: u64, b: u64) -> u64 {
    let mut x = a ^ b.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 32;
    x = x.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    x ^ (x >> 32)
}

/// A workload's graph and transactions, an input event for each
/// transaction, and the filters' pass rate.
#[derive(Clone)]
pub struct Fixture {
    pub graph: Graph,
    pub txs: Vec<Tx>,
    pub events: Vec<(Id, u64)>,
    pub pass: u32,
}

impl Fixture {
    pub fn new(name: &str, pass: u32) -> Fixture {
        let w = workload(name);
        let mut rng = Rng::new(0x5eed_5005);
        let events = w
            .txs
            .iter()
            .map(|_| (rng.below(INPUTS), rng.next_u64()))
            .collect();
        Fixture {
            graph: w.graph,
            txs: w.txs,
            events,
            pass,
        }
    }

    /// Moves across all transactions.
    pub fn moves(&self) -> usize {
        self.txs.iter().map(|t| t.moves.len()).sum()
    }

    /// Nodes built during the transactions.
    pub fn built(&self) -> usize {
        self.txs
            .iter()
            .flat_map(|t| &t.builds)
            .map(|b| b.len())
            .sum()
    }
}

/// What a transaction did, the same whichever engine ran it: how many nodes
/// fired, an order-independent digest of which and with what values, and
/// whether its moves were accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub fired: u32,
    pub digest: u64,
    pub accepted: bool,
}

/// The nodes' slots, shared by every scheduler.
#[derive(Clone)]
pub struct State {
    /// A node's value at `tx`, valid when `stamp` is `tx`.
    value: Vec<u64>,
    stamp: Vec<u32>,
    /// A node's committed value.
    current: Vec<u64>,
    tx: u32,
    /// The nodes that fired at `tx`, for commit.
    fired: Vec<Id>,
    digest: u64,
    /// A filter passes when the top ten bits of its value are below this.
    threshold: u32,
}

impl State {
    fn new(len: usize, pass: u32) -> State {
        let mut me = State {
            value: Vec::new(),
            stamp: Vec::new(),
            current: Vec::new(),
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
        self.current.extend((old..len).map(|n| mix(n as u64, 0)));
    }

    /// A new transaction, with `input` fired with `value`.
    #[inline]
    fn begin(&mut self, input: Id, value: u64) {
        self.tx += 1;
        self.fired.clear();
        self.digest = 0;
        self.value[input as usize] = value;
        self.fire(input);
    }

    /// The fired nodes' values become the committed ones.
    #[inline]
    fn commit(&mut self) {
        for &n in &self.fired {
            self.current[n as usize] = self.value[n as usize];
        }
    }

    /// Slots for the nodes built, and what the transaction did.
    #[inline]
    fn finish(&mut self, len: usize, accepted: bool) -> Outcome {
        self.grow(len);
        Outcome {
            fired: self.fired.len() as u32,
            digest: self.digest,
            accepted,
        }
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

    /// Evaluates `n` from its dependencies' slots; true if it fired. The
    /// same code for every scheduler and either layout, so they differ only
    /// in which nodes they call it on, in what order, and where the edges
    /// are read from.
    #[inline]
    pub fn eval<A: Adjacency>(&mut self, g: &A, n: Id) -> bool {
        let deps = g.deps(n);
        // The cheap check a quiet node does and nothing else.
        if !deps.iter().any(|&d| self.has_fired(d)) {
            // An input has no dependencies; only the one sent to fired.
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
            filter |= d <= ENTRY;
        }
        let v = mix(acc, n as u64);
        if filter && (v >> 54) as u32 >= self.threshold {
            return false;
        }
        self.value[n as usize] = v;
        self.fire(n);
        true
    }
}

/// Where a scheduler reads the edges from: the relink probe's [`Graph`],
/// a `Vec<Vec<Id>>` per direction, or [`Flat`]'s one edge array per
/// direction.
pub trait Adjacency {
    fn deps(&self, n: Id) -> &[Id];
    fn dependents(&self, n: Id) -> &[Id];
    fn nodes(&self) -> usize;
}

impl Adjacency for Graph {
    #[inline]
    fn deps(&self, n: Id) -> &[Id] {
        &self.deps[n as usize]
    }

    #[inline]
    fn dependents(&self, n: Id) -> &[Id] {
        &self.dependents[n as usize]
    }

    #[inline]
    fn nodes(&self) -> usize {
        self.len()
    }
}

/// How a transaction's region is evaluated, given the graph and the
/// checker that keeps whatever order the scheduler needs.
pub trait Schedule<C>: Clone + Default {
    fn evaluate(&mut self, st: &mut State, run: &Run<C>, input: Id);
    /// Nodes the last transaction marked, or popped.
    fn visited(&self) -> u32;
}

/// RFD 5: a depth-first mark over dependents from the input, then a flat
/// loop over the reverse post-order. Needs no order kept.
#[derive(Clone, Default)]
pub struct Mark {
    mark: Vec<u32>,
    order: Vec<Id>,
    stack: Vec<(Id, u32)>,
}

impl Mark {
    fn run<A: Adjacency>(&mut self, st: &mut State, g: &A, input: Id) {
        let tx = st.tx;
        self.mark.resize(g.nodes(), 0);
        self.order.clear();
        self.mark[input as usize] = tx;
        self.stack.push((input, 0));
        while let Some(top) = self.stack.last_mut() {
            let (n, k) = *top;
            if let Some(&d) = g.dependents(n).get(k as usize) {
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
        for k in (0..self.order.len()).rev() {
            st.eval(g, self.order[k]);
        }
    }
}

impl<C> Schedule<C> for Mark {
    fn evaluate(&mut self, st: &mut State, run: &Run<C>, input: Id) {
        self.run(st, &run.graph, input);
    }

    fn visited(&self) -> u32 {
        self.order.len() as u32
    }
}

/// A min-queue of nodes keyed by `u64`, popped in key order. Keys are
/// unique, and never pushed below the last key popped.
pub trait Queue: Clone + Default {
    fn push(&mut self, key: u64, n: Id);
    fn pop(&mut self) -> Option<Id>;
    /// Empty; ready for keys from zero up.
    fn reset(&mut self);
}

/// A radix heap: bucket `i > 0` holds the keys whose highest bit differing
/// from the last key popped is bit `i - 1`, and bucket 0 the key equal to
/// it. A pop from an empty bucket 0 takes the lowest non-empty bucket, makes
/// its least key the last, and spreads the rest over lower buckets.
#[derive(Clone)]
pub struct Radix {
    buckets: Vec<Vec<(u64, Id)>>,
    /// Bit `i` set when bucket `i` is non-empty.
    full: u128,
    last: u64,
}

impl Default for Radix {
    fn default() -> Self {
        Radix {
            buckets: vec![Vec::new(); 65],
            full: 0,
            last: 0,
        }
    }
}

impl Radix {
    #[inline]
    fn slot(&self, key: u64) -> usize {
        (64 - (key ^ self.last).leading_zeros()) as usize
    }

    #[inline]
    fn put(&mut self, key: u64, n: Id) {
        let b = self.slot(key);
        self.buckets[b].push((key, n));
        self.full |= 1 << b;
    }
}

impl Queue for Radix {
    #[inline]
    fn push(&mut self, key: u64, n: Id) {
        debug_assert!(key > self.last || self.full == 0);
        self.put(key, n);
    }

    fn pop(&mut self) -> Option<Id> {
        if self.full == 0 {
            return None;
        }
        let b = self.full.trailing_zeros() as usize;
        if b > 0 {
            let mut items = std::mem::take(&mut self.buckets[b]);
            self.full &= !(1 << b);
            self.last = items.iter().map(|&(k, _)| k).min().expect("non-empty");
            // Every key here now differs from the last below bit b - 1, so
            // each lands in a lower bucket, and the least in bucket 0.
            for &(k, n) in &items {
                self.put(k, n);
            }
            items.clear();
            self.buckets[b] = items;
        }
        let (_, n) = self.buckets[0].pop().expect("bucket 0 holds the least");
        if self.buckets[0].is_empty() {
            self.full &= !1;
        }
        Some(n)
    }

    fn reset(&mut self) {
        debug_assert_eq!(self.full, 0);
        self.last = 0;
    }
}

/// A binary heap over `(key, node)`.
#[derive(Clone, Default)]
pub struct Heap(BinaryHeap<Reverse<(u64, Id)>>);

impl Queue for Heap {
    #[inline]
    fn push(&mut self, key: u64, n: Id) {
        self.0.push(Reverse((key, n)));
    }

    #[inline]
    fn pop(&mut self) -> Option<Id> {
        self.0.pop().map(|Reverse((_, n))| n)
    }

    fn reset(&mut self) {}
}

/// Pops nodes in the order-maintenance list's label order, and pushes a
/// node's dependents only when it fires, once each per transaction.
#[derive(Clone, Default)]
pub struct Queued<Q> {
    queue: Q,
    queued: Vec<u32>,
    popped: u32,
}

impl<Q: Queue> Queued<Q> {
    #[inline]
    fn push_dependents<A: Adjacency>(&mut self, tx: u32, g: &A, order: &TwoWay, n: Id) {
        for &d in g.dependents(n) {
            if self.queued[d as usize] != tx {
                self.queued[d as usize] = tx;
                self.queue.push(order.order.label(d), d);
            }
        }
    }

    fn run<A: Adjacency>(&mut self, st: &mut State, g: &A, order: &TwoWay, input: Id) {
        self.queued.resize(g.nodes(), 0);
        self.queue.reset();
        self.popped = 0;
        self.push_dependents(st.tx, g, order, input);
        while let Some(n) = self.queue.pop() {
            self.popped += 1;
            if st.eval(g, n) {
                self.push_dependents(st.tx, g, order, n);
            }
        }
    }
}

impl<Q: Queue> Schedule<TwoWay> for Queued<Q> {
    fn evaluate(&mut self, st: &mut State, run: &Run<TwoWay>, input: Id) {
        self.run(st, &run.graph, &run.checker, input);
    }

    fn visited(&self) -> u32 {
        self.popped
    }
}

/// A graph, the checker that keeps its order, the slots, and a scheduler.
#[derive(Clone)]
pub struct Engine<C, S> {
    pub run: Run<C>,
    pub state: State,
    pub sched: S,
}

/// RFD 5 as written: the mark and loop, and the walk at each move.
pub type Walked = Engine<Baseline, Mark>;
/// The mark and loop, with the small-side order as the relink check.
pub type MarkOm = Engine<TwoWay, Mark>;
/// A radix heap over the small-side order's labels.
pub type RadixQueue = Engine<TwoWay, Queued<Radix>>;
/// A binary heap over the same labels.
pub type HeapQueue = Engine<TwoWay, Queued<Heap>>;

impl<C: Checker, S: Schedule<C>> Engine<C, S> {
    /// The fixture's graph before its first transaction, with the
    /// checker's initial order built.
    pub fn new(f: &Fixture) -> Self {
        Engine {
            run: Run {
                graph: f.graph.clone(),
                checker: C::new(&f.graph),
            },
            state: State::new(f.graph.len(), f.pass),
            sched: S::default(),
        }
    }

    /// One transaction: `input` fires with `value` and the region is
    /// evaluated; the values commit; the subgraphs built during it are
    /// built; the switches move and the check accepts or refuses them.
    pub fn tx(&mut self, t: &Tx, (input, value): (Id, u64)) -> Outcome {
        let st = &mut self.state;
        st.begin(input, value);
        self.sched.evaluate(st, &self.run, input);
        st.commit();
        let accepted = self.run.tx(t);
        st.finish(self.run.graph.len(), accepted)
    }

    /// Every transaction of `txs` with its event; the sum of the digests.
    pub fn all(&mut self, txs: &[Tx], events: &[(Id, u64)]) -> u64 {
        txs.iter()
            .zip(events)
            .fold(0, |acc, (t, &e)| acc.wrapping_add(self.tx(t, e).digest))
    }

    /// Nodes the last transaction marked, or popped.
    pub fn visited(&self) -> u32 {
        self.sched.visited()
    }

    /// The committed values, to compare engines after a run.
    pub fn values(&self) -> &[u64] {
        &self.state.current
    }
}

/// Runs every engine over the fixture and panics unless each transaction
/// fired the same nodes with the same values and got the same verdict, and
/// the committed values end the same.
pub fn agree(f: &Fixture) {
    let mut walked = Walked::new(f);
    let mut mark_om = MarkOm::new(f);
    let mut radix = RadixQueue::new(f);
    let mut heap = HeapQueue::new(f);
    for (k, (t, &e)) in f.txs.iter().zip(&f.events).enumerate() {
        let o = walked.tx(t, e);
        let others = [mark_om.tx(t, e), radix.tx(t, e), heap.tx(t, e)];
        assert!(others.iter().all(|x| *x == o), "tx {k}: {o:?} {others:?}");
    }
    let v = walked.values();
    assert!(mark_om.values() == v && radix.values() == v && heap.values() == v);
}

// The same engines over flat adjacency.

/// One direction's edges in one array: each node's list is a slice of
/// `edges`, at `span`'s start and length, with room for `cap` edges. Laid
/// out in id order with no room to spare, it is compressed sparse row with
/// a length beside each offset. A push onto a full slice first moves it to
/// the end with twice the room (at least four), leaving a hole; once holes
/// are half the array, every slice is laid out afresh in id order, keeping
/// its room. A removal swaps the last edge into its place, as
/// `Vec::swap_remove` does, so a list keeps the nested graph's order.
#[derive(Clone, Default)]
pub struct Slices {
    edges: Vec<Id>,
    /// Where a node's slice starts, and its length: all a read touches.
    span: Vec<(u32, u32)>,
    cap: Vec<u32>,
    /// Edge slots in holes.
    dead: usize,
    /// Slices moved to the end.
    pub relocated: u64,
    /// Fresh layouts.
    pub compacted: u64,
}

impl Slices {
    #[inline]
    fn get(&self, n: Id) -> &[Id] {
        let (start, len) = self.span[n as usize];
        &self.edges[start as usize..(start + len) as usize]
    }

    /// A slice for a new node, with no room to spare.
    fn add(&mut self, list: &[Id]) {
        let start = self.edges.len() as u32;
        self.edges.extend_from_slice(list);
        self.span.push((start, list.len() as u32));
        self.cap.push(list.len() as u32);
    }

    fn push(&mut self, n: Id, x: Id) {
        let i = n as usize;
        if self.span[i].1 == self.cap[i] {
            self.relocate(i);
        }
        let (start, len) = self.span[i];
        self.edges[(start + len) as usize] = x;
        self.span[i].1 = len + 1;
    }

    fn remove(&mut self, n: Id, x: Id) {
        let (start, len) = self.span[n as usize];
        let list = &mut self.edges[start as usize..(start + len) as usize];
        let k = list.iter().position(|&e| e == x).expect("no such edge");
        list[k] = list[len as usize - 1];
        self.span[n as usize].1 = len - 1;
    }

    /// Moves slice `i` to the end with twice the room.
    fn relocate(&mut self, i: usize) {
        let (start, len) = self.span[i];
        let cap = (2 * self.cap[i]).max(4);
        let at = self.edges.len();
        self.edges
            .extend_from_within(start as usize..(start + len) as usize);
        self.edges.resize(at + cap as usize, 0);
        self.dead += self.cap[i] as usize;
        self.span[i].0 = at as u32;
        self.cap[i] = cap;
        self.relocated += 1;
        if 2 * self.dead > self.edges.len() {
            self.compact();
        }
    }

    /// Lays every slice out afresh in id order, keeping its room.
    fn compact(&mut self) {
        let mut edges = Vec::with_capacity(self.edges.len() - self.dead);
        for (span, &cap) in self.span.iter_mut().zip(&self.cap) {
            let (start, len) = *span;
            let at = edges.len();
            edges.extend_from_slice(&self.edges[start as usize..(start + len) as usize]);
            edges.resize(at + cap as usize, 0);
            span.0 = at as u32;
        }
        self.edges = edges;
        self.dead = 0;
        self.compacted += 1;
    }

    /// Edge slots in the array, live, spare or in holes.
    pub fn slots(&self) -> usize {
        self.edges.len()
    }

    /// Edges in the slices.
    pub fn live(&self) -> usize {
        self.span.iter().map(|&(_, len)| len as usize).sum()
    }
}

/// The graph's edges in flat arrays, one per direction, kept in step with
/// the nested [`Graph`] the checkers own: the engine's scheduler reads only
/// these. A switch move is an unlink and a link in each direction: in the
/// switch's dependencies, a slice of one, the old inner's removal leaves
/// room the new inner's push reuses; in the old inner's dependents a
/// search and a swap; in the new inner's a push, which moves the slice to
/// the end when it is full. A node built appends its dependencies and an
/// empty dependents slice, and pushes itself onto each dependency's.
#[derive(Clone, Default)]
pub struct Flat {
    pub deps: Slices,
    pub dependents: Slices,
}

impl Adjacency for Flat {
    #[inline]
    fn deps(&self, n: Id) -> &[Id] {
        self.deps.get(n)
    }

    #[inline]
    fn dependents(&self, n: Id) -> &[Id] {
        self.dependents.get(n)
    }

    #[inline]
    fn nodes(&self) -> usize {
        self.deps.span.len()
    }
}

impl Flat {
    /// The graph laid out in id order, with no room to spare.
    pub fn new(g: &Graph) -> Flat {
        let mut me = Flat::default();
        for n in 0..g.len() {
            me.deps.add(&g.deps[n]);
            me.dependents.add(&g.dependents[n]);
        }
        me
    }

    /// The nodes of `g` from `first` on were just built.
    pub fn built(&mut self, g: &Graph, first: Id) {
        for n in first..g.len() as Id {
            let deps = &g.deps[n as usize];
            for &d in deps {
                self.dependents.push(d, n);
            }
            self.deps.add(deps);
            self.dependents.add(&[]);
        }
    }

    /// Moves the switches as the checkers do: every unlink, then every
    /// link.
    pub fn relink(&mut self, moves: &[Move]) {
        for m in moves {
            self.deps.remove(m.switch, m.from);
            self.dependents.remove(m.from, m.switch);
        }
        for m in moves {
            self.deps.push(m.switch, m.to);
            self.dependents.push(m.to, m.switch);
        }
    }

    /// A transaction's builds and commit on `run`, then the same here: the
    /// builds, and the moves if they were accepted. On a refusal the
    /// checker has put the nested graph back, and nothing here moved.
    pub fn tx<C: Checker>(&mut self, run: &mut Run<C>, t: &Tx) -> bool {
        // `Run::tx`, split so the built nodes are copied before the moves.
        let first = run.graph.len() as Id;
        for b in &t.builds {
            run.build(b);
        }
        self.built(&run.graph, first);
        let accepted = run.checker.commit(&mut run.graph, &t.moves);
        if accepted {
            self.relink(&t.moves);
        }
        accepted
    }

    /// Every transaction; returns how many were refused.
    pub fn all<C: Checker>(&mut self, run: &mut Run<C>, txs: &[Tx]) -> usize {
        txs.iter().filter(|t| !self.tx(run, t)).count()
    }

    /// Whether every node has the same edges as in `g`, in any order.
    pub fn same_as(&self, g: &Graph) -> bool {
        let same = |a: &[Id], b: &[Id]| {
            let (mut a, mut b) = (a.to_vec(), b.to_vec());
            a.sort_unstable();
            b.sort_unstable();
            a == b
        };
        self.nodes() == g.len()
            && (0..g.len() as Id).all(|n| {
                same(self.deps(n), &g.deps[n as usize])
                    && same(self.dependents(n), &g.dependents[n as usize])
            })
    }
}

/// A scheduler over the flat arrays, with the checker for whatever order
/// it pops in.
pub trait FlatSchedule<C>: Clone + Default {
    fn evaluate(&mut self, st: &mut State, run: &Run<C>, flat: &Flat, input: Id);
    /// Nodes the last transaction marked, or popped.
    fn visited(&self) -> u32;
}

impl<C> FlatSchedule<C> for Mark {
    fn evaluate(&mut self, st: &mut State, _: &Run<C>, flat: &Flat, input: Id) {
        self.run(st, flat, input);
    }

    fn visited(&self) -> u32 {
        self.order.len() as u32
    }
}

impl<Q: Queue> FlatSchedule<TwoWay> for Queued<Q> {
    fn evaluate(&mut self, st: &mut State, run: &Run<TwoWay>, flat: &Flat, input: Id) {
        self.run(st, flat, &run.checker, input);
    }

    fn visited(&self) -> u32 {
        self.popped
    }
}

/// A bucket per Incremental height, popped by one climbing cursor: the
/// height-queue probe's `Buckets` line for line, over the flat arrays,
/// with its [`Heights`] as the checker.
#[derive(Clone, Default)]
pub struct Climb {
    buckets: Vec<Vec<Id>>,
    queued: Vec<u32>,
    tx: u32,
    popped: u32,
}

impl Climb {
    #[inline]
    fn push_dependents(&mut self, g: &Flat, height: &[u32], n: Id, mut top: usize) -> usize {
        for &d in g.dependents(n) {
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

impl FlatSchedule<Heights> for Climb {
    fn evaluate(&mut self, st: &mut State, run: &Run<Heights>, g: &Flat, input: Id) {
        let height = &run.checker.height;
        self.tx += 1;
        self.queued.resize(g.nodes(), 0);
        if self.buckets.len() <= run.checker.max as usize {
            self.buckets
                .resize(run.checker.max as usize + 1, Vec::new());
        }
        self.popped = 0;
        let mut h = height[input as usize] as usize + 1;
        let mut top = self.push_dependents(g, height, input, 0);
        while h <= top {
            let Some(n) = self.buckets[h].pop() else {
                h += 1;
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

/// [`Engine`] with the scheduler reading [`Flat`]: the checker still keeps
/// its order over the nested graph, and the flat arrays follow it.
#[derive(Clone)]
pub struct FlatEngine<C, S> {
    pub run: Run<C>,
    pub flat: Flat,
    pub state: State,
    pub sched: S,
}

/// The mark and loop, with the small-side order as the relink check.
pub type FlatMarkOm = FlatEngine<TwoWay, Mark>;
/// A radix heap over the small-side order's labels.
pub type FlatRadixQueue = FlatEngine<TwoWay, Queued<Radix>>;
/// A binary heap over the same labels.
pub type FlatHeapQueue = FlatEngine<TwoWay, Queued<Heap>>;
/// Incremental's heights and a bucket queue by them.
pub type FlatHeightQueue = FlatEngine<Heights, Climb>;

impl<C: Checker, S: FlatSchedule<C>> FlatEngine<C, S> {
    pub fn new(f: &Fixture) -> Self {
        FlatEngine {
            run: Run {
                graph: f.graph.clone(),
                checker: C::new(&f.graph),
            },
            flat: Flat::new(&f.graph),
            state: State::new(f.graph.len(), f.pass),
            sched: S::default(),
        }
    }

    /// As [`Engine::tx`], with the flat arrays following the commit.
    pub fn tx(&mut self, t: &Tx, (input, value): (Id, u64)) -> Outcome {
        let st = &mut self.state;
        st.begin(input, value);
        self.sched.evaluate(st, &self.run, &self.flat, input);
        st.commit();
        let accepted = self.flat.tx(&mut self.run, t);
        st.finish(self.run.graph.len(), accepted)
    }

    /// Every transaction of `txs` with its event; the sum of the digests.
    pub fn all(&mut self, txs: &[Tx], events: &[(Id, u64)]) -> u64 {
        txs.iter()
            .zip(events)
            .fold(0, |acc, (t, &e)| acc.wrapping_add(self.tx(t, e).digest))
    }

    /// Nodes the last transaction marked, or popped.
    pub fn visited(&self) -> u32 {
        self.sched.visited()
    }

    /// The committed values, to compare engines after a run.
    pub fn values(&self) -> &[u64] {
        &self.state.current
    }
}

/// [`agree`] for the flat engines: each transaction's fired set, values
/// and verdict the same as RFD 5 as written over the nested graph, and
/// the flat arrays holding the nested graph's edges after it.
pub fn agree_flat(f: &Fixture) {
    let mut walked = Walked::new(f);
    let mut mark = FlatMarkOm::new(f);
    let mut radix = FlatRadixQueue::new(f);
    let mut heap = FlatHeapQueue::new(f);
    let mut heights = FlatHeightQueue::new(f);
    for (k, (t, &e)) in f.txs.iter().zip(&f.events).enumerate() {
        let o = walked.tx(t, e);
        let others = [
            mark.tx(t, e),
            radix.tx(t, e),
            heap.tx(t, e),
            heights.tx(t, e),
        ];
        assert!(others.iter().all(|x| *x == o), "tx {k}: {o:?} {others:?}");
        debug_assert!(mark.flat.same_as(&mark.run.graph), "tx {k}");
        debug_assert!(heights.flat.same_as(&heights.run.graph), "tx {k}");
    }
    let v = walked.values();
    assert!(
        mark.values() == v && radix.values() == v && heap.values() == v && heights.values() == v
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radix_pops_in_key_order() {
        let mut rng = Rng::new(7);
        let mut q = Radix::default();
        for _ in 0..50 {
            q.reset();
            let mut keys: Vec<u64> = (0..200).map(|_| rng.next_u64() >> 2).collect();
            keys.sort_unstable();
            keys.dedup();
            // Monotone use: push a chunk out of order, pop one, push the
            // next chunk, all above what was popped.
            let mut got = Vec::new();
            for chunk in keys.chunks(7) {
                for &k in chunk.iter().rev() {
                    q.push(k, k as Id);
                }
                got.push(q.pop().unwrap());
            }
            while let Some(n) = q.pop() {
                got.push(n);
            }
            let want: Vec<Id> = keys.iter().map(|&k| k as Id).collect();
            assert_eq!(got, want);
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

    /// What each workload does per transaction, averaged over its
    /// transactions: the region the mark walks, how many nodes fired, the
    /// quiet fraction of the region, and how many nodes the queue popped.
    /// Run with `--nocapture`.
    #[test]
    fn counts() {
        println!(
            "{:<8} {:>4} {:>6} {:>6} {:>6} {:>8} {:>8} {:>7} {:>8}",
            "workload", "pass", "nodes", "moves", "built", "marked", "fired", "quiet%", "popped"
        );
        let mut txs = 0;
        for name in WORKLOADS {
            for pass in PASS {
                let f = Fixture::new(name, pass);
                agree(&f);
                txs = f.txs.len();
                let mut mark = MarkOm::new(&f);
                let mut queue = RadixQueue::new(&f);
                let (mut marked, mut fired, mut popped) = (0u64, 0u64, 0u64);
                for (t, &e) in f.txs.iter().zip(&f.events) {
                    fired += mark.tx(t, e).fired as u64;
                    marked += mark.visited() as u64;
                    queue.tx(t, e);
                    popped += queue.visited() as u64;
                }
                let n = f.txs.len() as f64;
                println!(
                    "{:<8} {:>4} {:>6} {:>6} {:>6} {:>8.1} {:>8.1} {:>7.1} {:>8.1}",
                    name,
                    pass,
                    f.graph.len(),
                    f.moves(),
                    f.built(),
                    marked as f64 / n,
                    fired as f64 / n,
                    100.0 * (1.0 - fired as f64 / marked as f64),
                    popped as f64 / n,
                );
            }
        }
        println!();
        println!("per transaction, over each workload's {txs} transactions: nodes marked by");
        println!("the depth-first mark, fired, the quiet share of the marked region, and");
        println!("popped by the label-ordered queue. pass is the filters' pass rate in");
        println!("percent; nodes is the graph before the first transaction; moves and");
        println!("built are totals over the workload.");
    }

    #[test]
    fn flat_engines_agree() {
        for name in WORKLOADS {
            for pass in [100, 20, 0] {
                agree_flat(&Fixture::new(name, pass));
            }
        }
    }

    /// After every transaction, refused or not, the flat arrays hold the
    /// nested graph's edges, under either checker.
    #[test]
    fn flat_follows_graph() {
        fn follow<C: Checker>(f: &Fixture) {
            let mut run = Run {
                graph: f.graph.clone(),
                checker: C::new(&f.graph),
            };
            let mut flat = Flat::new(&f.graph);
            assert!(flat.same_as(&run.graph));
            for (k, t) in f.txs.iter().enumerate() {
                flat.tx(&mut run, t);
                assert!(flat.same_as(&run.graph), "tx {k}");
            }
        }
        for name in WORKLOADS {
            let f = Fixture::new(name, 0);
            follow::<TwoWay>(&f);
            follow::<Heights>(&f);
        }
    }

    /// What the flat engines visit per transaction, the same as the nested
    /// ones (asserted), and what keeping the flat arrays did over each
    /// workload. Run with `--nocapture`.
    #[test]
    fn counts_flat() {
        use crate::rfd_0005_height_queue::HeightQueue;
        println!(
            "{:<8} {:>4} {:>8} {:>8} {:>7} {:>8} {:>8}",
            "workload", "pass", "marked", "fired", "quiet%", "labels", "heights"
        );
        let mut layout = Vec::new();
        for name in WORKLOADS {
            for pass in PASS {
                let f = Fixture::new(name, pass);
                agree_flat(&f);
                let mut mark = FlatMarkOm::new(&f);
                let mut heap = FlatHeapQueue::new(&f);
                let mut heights = FlatHeightQueue::new(&f);
                let mut nested = (MarkOm::new(&f), HeapQueue::new(&f), HeightQueue::new(&f));
                let (mut marked, mut fired, mut labels, mut raised) = (0u64, 0u64, 0u64, 0u64);
                for (t, &e) in f.txs.iter().zip(&f.events) {
                    fired += mark.tx(t, e).fired as u64;
                    heap.tx(t, e);
                    heights.tx(t, e);
                    nested.0.tx(t, e);
                    nested.1.tx(t, e);
                    nested.2.tx(t, e);
                    assert_eq!(mark.visited(), nested.0.visited());
                    assert_eq!(heap.visited(), nested.1.visited());
                    assert_eq!(heights.visited(), nested.2.visited());
                    marked += mark.visited() as u64;
                    labels += heap.visited() as u64;
                    raised += heights.visited() as u64;
                }
                let n = f.txs.len() as f64;
                println!(
                    "{:<8} {:>4} {:>8.1} {:>8.1} {:>7.1} {:>8.1} {:>8.1}",
                    name,
                    pass,
                    marked as f64 / n,
                    fired as f64 / n,
                    100.0 * (1.0 - fired as f64 / marked as f64),
                    labels as f64 / n,
                    raised as f64 / n,
                );
                if pass == PASS[0] {
                    let refused = Run {
                        graph: f.graph.clone(),
                        checker: Baseline::new(&f.graph),
                    }
                    .all(&f.txs);
                    layout.push((name, f.moves(), f.built(), refused, mark.flat.clone()));
                }
            }
        }
        println!();
        println!("per transaction, over each workload's 300 transactions: nodes marked by");
        println!("the depth-first mark over the flat arrays, fired, the quiet share of the");
        println!("marked region, and popped by the label heap and by the height buckets over");
        println!("the flat arrays. Each equals the nested engine's, transaction by transaction.");
        println!();
        println!(
            "{:<8} {:>6} {:>6} {:>7} | {:>6} {:>6} {:>6} | {:>6} {:>6} {:>6} {:>6}",
            "workload",
            "moves",
            "built",
            "refused",
            "live",
            "slots",
            "moved",
            "live",
            "slots",
            "moved",
            "fresh"
        );
        for (name, moves, built, refused, fl) in layout {
            let (d, o) = (&fl.deps, &fl.dependents);
            println!(
                "{:<8} {:>6} {:>6} {:>7} | {:>6} {:>6} {:>6} | {:>6} {:>6} {:>6} {:>6}",
                name,
                moves,
                built,
                refused,
                d.live(),
                d.slots(),
                d.relocated,
                o.live(),
                o.slots(),
                o.relocated,
                o.compacted,
            );
            assert_eq!(d.compacted, 0);
        }
        println!();
        println!("the flat arrays after each workload: moves and nodes built over it, and");
        println!("transactions refused (their moves are never applied to the arrays); then");
        println!("for dependencies and for dependents, edges live, slots in the array (live,");
        println!("spare room and holes), slices moved to the end for room, and (dependents;");
        println!("dependencies had none) fresh layouts once holes were half the array.");
    }
}
