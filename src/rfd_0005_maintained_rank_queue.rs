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

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::rfd_0005_bounded_relink_check::{Baseline, Checker, Graph, Id, Run, Tx, workload};
use crate::rfd_0005_heap_vs_mark_on_quiet_regions::Rng;
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
    /// same code for every scheduler, so they differ only in which nodes
    /// they call it on and in what order.
    #[inline]
    fn eval(&mut self, g: &Graph, n: Id) -> bool {
        let deps = &g.deps[n as usize];
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

impl<C> Schedule<C> for Mark {
    fn evaluate(&mut self, st: &mut State, run: &Run<C>, input: Id) {
        let g = &run.graph;
        let tx = st.tx;
        self.mark.resize(g.len(), 0);
        self.order.clear();
        self.mark[input as usize] = tx;
        self.stack.push((input, 0));
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
        for k in (0..self.order.len()).rev() {
            st.eval(g, self.order[k]);
        }
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
    fn push_dependents(&mut self, tx: u32, run: &Run<TwoWay>, n: Id) {
        for &d in &run.graph.dependents[n as usize] {
            if self.queued[d as usize] != tx {
                self.queued[d as usize] = tx;
                self.queue.push(run.checker.order.label(d), d);
            }
        }
    }
}

impl<Q: Queue> Schedule<TwoWay> for Queued<Q> {
    fn evaluate(&mut self, st: &mut State, run: &Run<TwoWay>, input: Id) {
        self.queued.resize(run.graph.len(), 0);
        self.queue.reset();
        self.popped = 0;
        self.push_dependents(st.tx, run, input);
        while let Some(n) = self.queue.pop() {
            self.popped += 1;
            if st.eval(&run.graph, n) {
                self.push_dependents(st.tx, run, n);
            }
        }
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
        st.tx += 1;
        st.fired.clear();
        st.digest = 0;
        st.value[input as usize] = value;
        st.fire(input);
        self.sched.evaluate(st, &self.run, input);
        for &n in &st.fired {
            st.current[n as usize] = st.value[n as usize];
        }
        let accepted = self.run.tx(t);
        st.grow(self.run.graph.len());
        Outcome {
            fired: st.fired.len() as u32,
            digest: st.digest,
            accepted,
        }
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
}
