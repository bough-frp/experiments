//! Does a queue that visits only firing nodes beat RFD 5's mark plus flat
//! loop when most of the marked region stays quiet, and at what quiet
//! fraction?
//!
//! RFD 5 marks every node reachable over dependents from the fired inputs,
//! with a depth-first walk, and evaluates the region in reverse post-order.
//! A node none of whose inputs fired does a cheap check and nothing else.
//! It rejects a heap because a heap adds a log factor when the region is
//! most of the graph. The alternative (Acar's change propagation, Jane
//! Street's Incremental) is a priority queue keyed by a topological rank
//! that pushes a node's dependents only when the node fires, so a quiet
//! subregion behind a filter that rejected, or a cell that didn't step, is
//! never visited past its first node.
//!
//! This module is a model of both over one graph: streams (input, map,
//! filter with a tunable pass rate, merge, snapshot, steps) and cells
//! (hold, read-through `map_cell`), with a few multiply-xors of work per
//! node. A snapshot's read of a hold is not a dependency. Three schedulers
//! run the same event sequence, and must produce the same fired set, the
//! same values and the same holds (`agree` checks it):
//!
//! - `Mark`, the baseline: RFD 5's depth-first mark and flat loop.
//! - `Heap`: a binary heap keyed by `(height, id)`, fire-only pushes, a
//!   dedup stamp per node.
//! - `Bucket`: a bucket queue indexed by height, as Incremental's
//!   recompute heap is, with the same pushes and stamp. Pushing is O(1) and
//!   popping scans empty buckets, at most the graph's height per
//!   transaction, so it is the queue with no log factor to pay; the binary
//!   heap is the one RFD 5 argued against. Both are measured.
//!
//! The ranks are static: every node's height is computed once when the
//! graph is built, and there are no switches, no `construct` and no nodes
//! built during a transaction. So the comparison isolates scheduling. It
//! leaves out what a maintained rank costs when a switch moves (the relink
//! probe's question), and the pull RFD 5 falls back on.
//!
//! Workloads: `ui` is a pointer-like input fanned out to `k` widgets, each
//! a filter on the input (is this event mine?), a snapshot of the widget's
//! own hold, a map, the hold, and `f` read-through cells over the hold,
//! each with a steps and a map, merged into the widget's output. The
//! widget outputs merge in a tree into an app-level hold with a few cells
//! of its own. The filters' pass rate sets how much of the marked region
//! stays quiet. `frame` is RFD 5's frame shape: a tick into `w` snapshots
//! of `w` entity holds, six layers of maps and two-way merges, and the
//! holds again, all of which fires every tick.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// A node's index. Nodes are built in dependency order, so every
/// dependency has a smaller id than its dependent.
pub type Id = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Input,
    /// `arg` is mixed into the value.
    Map,
    /// Passes a value when its hash, in 1/1024ths, is below `arg`.
    Filter,
    /// Fires when any dependency fired, with the sum of those that did.
    Merge,
    /// Reads hold `arg`'s value from before the instant. Not a dependency.
    Snapshot,
    /// A cell that stores its stream's value at commit.
    Hold,
    /// A read-through cell: no storage, steps when its cell stepped, and a
    /// read reads through with `arg` mixed in.
    MapCell,
    /// Fires when its cell stepped, with the cell's value after the
    /// instant.
    Steps,
}

/// Which scheduler runs a transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sched {
    Mark,
    Heap,
    Bucket,
}

impl Sched {
    pub const ALL: [Sched; 3] = [Sched::Mark, Sched::Heap, Sched::Bucket];

    pub fn name(self) -> &'static str {
        match self {
            Sched::Mark => "baseline",
            Sched::Heap => "heap",
            Sched::Bucket => "bucket",
        }
    }
}

/// What a transaction did, the same whichever scheduler ran it: how many
/// nodes fired, and an order-independent digest of which and with what.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub fired: u32,
    pub digest: u64,
}

/// A few multiply-xors: the fixed node-local work, and the hash.
#[inline]
fn mix(a: u64, b: u64) -> u64 {
    let mut x = a ^ b.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 32;
    x = x.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    x ^ (x >> 32)
}

/// SplitMix64: the graphs and the events are generated from a fixed seed.
#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: u32) -> u32 {
        (self.next_u64() % n as u64) as u32
    }
}

/// Builds a graph as nodes and dependency lists, then freezes it into an
/// `Engine` with both adjacency directions flattened.
#[derive(Default)]
pub struct Builder {
    kind: Vec<Kind>,
    arg: Vec<u32>,
    deps: Vec<Vec<Id>>,
}

impl Builder {
    pub fn node(&mut self, kind: Kind, arg: u32, deps: &[Id]) -> Id {
        let n = self.kind.len() as Id;
        assert!(deps.iter().all(|&d| d < n), "dependencies come first");
        assert_eq!(kind == Kind::Input, deps.is_empty());
        self.kind.push(kind);
        self.arg.push(arg);
        self.deps.push(deps.to_vec());
        n
    }

    pub fn freeze(self) -> Engine {
        let len = self.kind.len();
        let mut dep_at = vec![0u32];
        let mut deps = Vec::new();
        let mut height = vec![0u32; len];
        let mut count = vec![0u32; len];
        for (n, ds) in self.deps.iter().enumerate() {
            deps.extend_from_slice(ds);
            dep_at.push(deps.len() as u32);
            height[n] = ds
                .iter()
                .map(|&d| height[d as usize] + 1)
                .max()
                .unwrap_or(0);
            for &d in ds {
                count[d as usize] += 1;
            }
        }
        let mut out_at = vec![0u32; len + 1];
        for n in 0..len {
            out_at[n + 1] = out_at[n] + count[n];
        }
        let mut fill = out_at.clone();
        let mut dependents = vec![0; deps.len()];
        for (n, ds) in self.deps.iter().enumerate() {
            for &d in ds {
                dependents[fill[d as usize] as usize] = n as Id;
                fill[d as usize] += 1;
            }
        }
        let current = (0..len as u64).map(|n| mix(n, 0)).collect();
        let max_height = height.iter().copied().max().unwrap_or(0) as usize;
        Engine {
            kind: self.kind,
            arg: self.arg,
            dep_at,
            deps,
            out_at,
            dependents,
            height,
            value: vec![0; len],
            stamp: vec![0; len],
            current,
            tx: 0,
            fired: 0,
            digest: 0,
            stepped: Vec::new(),
            mark: vec![0; len],
            order: Vec::new(),
            stack: Vec::new(),
            queued: vec![0; len],
            heap: BinaryHeap::new(),
            buckets: vec![Vec::new(); max_height + 1],
            visited: 0,
        }
    }
}

/// The frozen graph, its slots, and each scheduler's reused scratch.
#[derive(Clone)]
pub struct Engine {
    kind: Vec<Kind>,
    arg: Vec<u32>,
    dep_at: Vec<u32>,
    deps: Vec<Id>,
    out_at: Vec<u32>,
    dependents: Vec<Id>,
    /// 0 for an input, else one more than the highest dependency: the
    /// static topological rank both queues key by.
    height: Vec<u32>,

    /// A node's value at `tx`, valid when `stamp` is `tx`. For a hold,
    /// its value after the instant.
    value: Vec<u64>,
    stamp: Vec<u32>,
    /// A hold's value before the instant.
    current: Vec<u64>,
    tx: u32,
    fired: u32,
    digest: u64,
    /// Holds that stepped, for commit.
    stepped: Vec<Id>,

    // Mark: stamps, the post-order, and the walk's stack.
    mark: Vec<u32>,
    order: Vec<Id>,
    stack: Vec<(Id, u32)>,

    // The queues: a dedup stamp, the heap, the buckets, and how many nodes
    // the last transaction popped.
    queued: Vec<u32>,
    heap: BinaryHeap<Reverse<u64>>,
    buckets: Vec<Vec<Id>>,
    visited: u32,
}

impl Engine {
    pub fn len(&self) -> usize {
        self.kind.len()
    }

    pub fn is_empty(&self) -> bool {
        self.kind.is_empty()
    }

    /// How many nodes the last `Mark` transaction marked.
    pub fn marked(&self) -> u32 {
        self.order.len() as u32
    }

    /// How many nodes the last `Heap` or `Bucket` transaction popped.
    pub fn visited(&self) -> u32 {
        self.visited
    }

    pub fn max_height(&self) -> u32 {
        self.buckets.len() as u32 - 1
    }

    /// The holds' values between transactions.
    pub fn holds(&self) -> Vec<u64> {
        (0..self.len())
            .filter(|&n| self.kind[n] == Kind::Hold)
            .map(|n| self.current[n])
            .collect()
    }

    /// One transaction: `input` fires with `value`, then commit.
    pub fn run(&mut self, sched: Sched, input: Id, value: u64) -> Outcome {
        self.begin(input, value);
        match sched {
            Sched::Mark => self.by_mark(input),
            Sched::Heap => self.by_heap(input),
            Sched::Bucket => self.by_bucket(input),
        }
        for &h in &self.stepped {
            self.current[h as usize] = self.value[h as usize];
        }
        self.stepped.clear();
        Outcome {
            fired: self.fired,
            digest: self.digest,
        }
    }

    fn begin(&mut self, input: Id, value: u64) {
        self.tx += 1;
        self.fired = 0;
        self.digest = 0;
        let i = input as usize;
        assert_eq!(self.kind[i], Kind::Input);
        self.value[i] = value;
        self.fire(input);
    }

    #[inline]
    fn fire(&mut self, n: Id) {
        let i = n as usize;
        self.stamp[i] = self.tx;
        self.fired += 1;
        self.digest = self.digest.wrapping_add(mix(self.value[i], n as u64));
    }

    #[inline]
    fn has_fired(&self, n: Id) -> bool {
        self.stamp[n as usize] == self.tx
    }

    /// A cell's value after the instant.
    fn read_post(&self, c: Id) -> u64 {
        let i = c as usize;
        match self.kind[i] {
            Kind::Hold if self.has_fired(c) => self.value[i],
            Kind::Hold => self.current[i],
            Kind::MapCell => {
                let d = self.deps[self.dep_at[i] as usize];
                mix(self.read_post(d), self.arg[i] as u64)
            }
            k => unreachable!("{k:?} is not a cell"),
        }
    }

    /// Evaluates `n` from its dependencies' slots; true if it fired. The
    /// same code for every scheduler, so they differ only in which nodes
    /// they call it on.
    #[inline]
    fn eval(&mut self, n: Id) -> bool {
        let i = n as usize;
        let kind = self.kind[i];
        if kind == Kind::Input {
            // Only the fired input is ever in a region, and begin fired it.
            return self.has_fired(n);
        }
        let first = self.dep_at[i] as usize;
        let d = self.deps[first];
        let value = match kind {
            Kind::Merge => {
                let mut any = false;
                let mut sum = 0u64;
                for k in first..self.dep_at[i + 1] as usize {
                    let d = self.deps[k];
                    if self.has_fired(d) {
                        any = true;
                        sum = sum.wrapping_add(self.value[d as usize]);
                    }
                }
                if !any {
                    return false;
                }
                sum
            }
            // Every other kind has one dependency, and is quiet unless it
            // fired: the cheap check.
            _ if !self.has_fired(d) => return false,
            Kind::Map => mix(self.value[d as usize], self.arg[i] as u64),
            Kind::Filter => {
                let v = self.value[d as usize];
                if (mix(v, n as u64) >> 54) as u32 >= self.arg[i] {
                    return false;
                }
                v
            }
            Kind::Snapshot => mix(self.value[d as usize], self.current[self.arg[i] as usize]),
            Kind::Hold => {
                self.stepped.push(n);
                self.value[d as usize]
            }
            Kind::MapCell => 0,
            Kind::Steps => self.read_post(d),
            Kind::Input => unreachable!(),
        };
        self.value[i] = value;
        self.fire(n);
        true
    }

    /// RFD 5: a depth-first mark over dependents, then a flat loop over the
    /// reverse post-order.
    fn by_mark(&mut self, input: Id) {
        let tx = self.tx;
        self.order.clear();
        self.mark[input as usize] = tx;
        self.stack.push((input, self.out_at[input as usize]));
        while let Some(top) = self.stack.last_mut() {
            let (n, k) = *top;
            if k < self.out_at[n as usize + 1] {
                top.1 += 1;
                let d = self.dependents[k as usize];
                if self.mark[d as usize] != tx {
                    self.mark[d as usize] = tx;
                    self.stack.push((d, self.out_at[d as usize]));
                }
            } else {
                self.order.push(n);
                self.stack.pop();
            }
        }
        for k in (0..self.order.len()).rev() {
            let n = self.order[k];
            self.eval(n);
        }
    }

    /// A binary heap keyed by `(height, id)`: pops in topological order,
    /// and a node's dependents go in only when it fires.
    fn by_heap(&mut self, input: Id) {
        self.visited = 0;
        self.push_dependents_heap(input);
        while let Some(Reverse(key)) = self.heap.pop() {
            let n = key as Id;
            self.visited += 1;
            if self.eval(n) {
                self.push_dependents_heap(n);
            }
        }
    }

    #[inline]
    fn push_dependents_heap(&mut self, n: Id) {
        let tx = self.tx;
        for k in self.out_at[n as usize]..self.out_at[n as usize + 1] {
            let d = self.dependents[k as usize];
            if self.queued[d as usize] != tx {
                self.queued[d as usize] = tx;
                let key = ((self.height[d as usize] as u64) << 32) | d as u64;
                self.heap.push(Reverse(key));
            }
        }
    }

    /// A bucket per height. A dependent is always higher than the node
    /// that pushed it, so one cursor climbing from the bottom pops in
    /// topological order, and stops at the highest bucket pushed to.
    fn by_bucket(&mut self, input: Id) {
        self.visited = 0;
        let mut top = self.push_dependents_bucket(input, 0);
        let mut h = 0;
        while h <= top {
            let Some(n) = self.buckets[h].pop() else {
                h += 1;
                continue;
            };
            self.visited += 1;
            if self.eval(n) {
                top = self.push_dependents_bucket(n, top);
            }
        }
    }

    #[inline]
    fn push_dependents_bucket(&mut self, n: Id, mut top: usize) -> usize {
        let tx = self.tx;
        for k in self.out_at[n as usize]..self.out_at[n as usize + 1] {
            let d = self.dependents[k as usize];
            if self.queued[d as usize] != tx {
                self.queued[d as usize] = tx;
                let h = self.height[d as usize] as usize;
                self.buckets[h].push(d);
                top = top.max(h);
            }
        }
        top
    }
}

/// The UI shape: one input fanned out to `k` widgets with `f` read-through
/// cells each; filters pass `pass` percent of events.
pub fn ui(k: u32, f: u32, pass: u32) -> Engine {
    let threshold = pass * 1024 / 100;
    let mut b = Builder::default();
    let input = b.node(Kind::Input, 0, &[]);
    let mut outputs = Vec::new();
    for w in 0..k {
        let filter = b.node(Kind::Filter, threshold, &[input]);
        // The snapshot reads the widget's own hold, built below; the read
        // is not an edge, so the id is patched in after.
        let snap = b.node(Kind::Snapshot, 0, &[filter]);
        let map = b.node(Kind::Map, w, &[snap]);
        let hold = b.node(Kind::Hold, 0, &[map]);
        b.arg[snap as usize] = hold;
        let mut outs = Vec::new();
        for j in 0..f {
            let cell = b.node(Kind::MapCell, j, &[hold]);
            let steps = b.node(Kind::Steps, 0, &[cell]);
            outs.push(b.node(Kind::Map, j, &[steps]));
        }
        outputs.push(b.node(Kind::Merge, 0, &outs));
    }
    while outputs.len() > 1 {
        outputs = outputs
            .chunks(2)
            .map(|pair| match pair {
                [a, c] => b.node(Kind::Merge, 0, &[*a, *c]),
                [a] => *a,
                _ => unreachable!(),
            })
            .collect();
    }
    let app = b.node(Kind::Hold, 0, &[outputs[0]]);
    for j in 0..4 {
        let cell = b.node(Kind::MapCell, j, &[app]);
        let steps = b.node(Kind::Steps, 0, &[cell]);
        b.node(Kind::Map, j, &[steps]);
    }
    b.freeze()
}

/// RFD 5's frame shape: a tick into `w` snapshots of `w` entity holds, six
/// layers of maps and two-way merges, and the holds. All of it fires.
pub fn frame(w: u32, seed: u64) -> Engine {
    let mut rng = Rng::new(seed);
    let mut b = Builder::default();
    let tick = b.node(Kind::Input, 0, &[]);
    let snaps: Vec<Id> = (0..w).map(|_| b.node(Kind::Snapshot, 0, &[tick])).collect();
    let mut layer = snaps.clone();
    for _ in 0..6 {
        layer = (0..w as usize)
            .map(|i| {
                if rng.below(2) == 0 {
                    b.node(Kind::Map, i as u32, &[layer[i]])
                } else {
                    let other = layer[rng.below(w) as usize];
                    b.node(Kind::Merge, 0, &[layer[i], other])
                }
            })
            .collect();
    }
    for (i, &s) in snaps.iter().enumerate() {
        let hold = b.node(Kind::Hold, 0, &[layer[i]]);
        b.arg[s as usize] = hold;
    }
    b.freeze()
}

/// The filter pass rates the UI sweep runs, in percent.
pub const PASS: [u32; 10] = [100, 90, 75, 50, 25, 10, 5, 2, 1, 0];

/// The frame widths.
pub const WIDTH: [u32; 2] = [64, 1024];

/// How many events a fixture cycles through. The counts, the instruction
/// counts and the wall-clock times all run exactly this cycle, so the quiet
/// fraction the counts report is the one the benches measured.
pub const CYCLE: usize = 64;

/// A graph and an event sequence, cycled.
#[derive(Clone)]
pub struct Fixture {
    pub engine: Engine,
    events: Vec<u64>,
    next: usize,
}

impl Fixture {
    fn new(engine: Engine, seed: u64) -> Fixture {
        let mut rng = Rng::new(seed);
        Fixture {
            engine,
            events: (0..CYCLE).map(|_| rng.next_u64()).collect(),
            next: 0,
        }
    }

    /// Eight widgets of eight cells: 253 nodes, a screen.
    pub fn ui_small(pass: u32) -> Fixture {
        Fixture::new(ui(8, 8, pass), 1)
    }

    /// 128 widgets of 24 cells: 9,997 nodes, a large view.
    pub fn ui_large(pass: u32) -> Fixture {
        Fixture::new(ui(128, 24, pass), 2)
    }

    /// `w` entities, `8 w + 1` nodes.
    pub fn frame(w: u32) -> Fixture {
        Fixture::new(frame(w, 3), 3)
    }

    /// The next event, as one transaction.
    pub fn step(&mut self, sched: Sched) -> Outcome {
        let v = self.events[self.next];
        self.next = (self.next + 1) % self.events.len();
        self.engine.run(sched, 0, v)
    }

    /// `n` transactions; the sum of their digests.
    pub fn steps(&mut self, sched: Sched, n: usize) -> u64 {
        (0..n).fold(0, |acc, _| acc.wrapping_add(self.step(sched).digest))
    }
}

/// Runs every scheduler over the same `n` events from the same fixture and
/// panics unless each transaction fired the same nodes with the same
/// values, and the holds end the same.
pub fn agree(f: &Fixture, n: usize) {
    let mut runs: Vec<Fixture> = Sched::ALL.iter().map(|_| f.clone()).collect();
    for t in 0..n {
        let outs: Vec<Outcome> = Sched::ALL
            .iter()
            .zip(runs.iter_mut())
            .map(|(&s, r)| r.step(s))
            .collect();
        assert!(
            outs.iter().all(|o| *o == outs[0]),
            "transaction {t}: {outs:?}"
        );
    }
    let holds = runs[0].engine.holds();
    assert!(runs.iter().all(|r| r.engine.holds() == holds));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedulers_agree() {
        for pass in PASS {
            agree(&Fixture::ui_small(pass), 4 * CYCLE);
            agree(&Fixture::ui_large(pass), 64);
        }
        for w in WIDTH {
            agree(&Fixture::frame(w), 64);
        }
    }

    /// What each workload does per transaction, averaged over one cycle of
    /// events: nodes, marked region, fired, the quiet fraction of the
    /// marked region, and how many nodes the queues popped.
    #[test]
    fn counts() {
        const N: usize = CYCLE;
        println!(
            "{:<9} {:>5} {:>6} {:>6} {:>8} {:>7} {:>7} {:>7}",
            "shape", "param", "nodes", "height", "marked", "fired", "quiet%", "popped"
        );
        let mut rows: Vec<(&str, u32, Fixture)> = Vec::new();
        for pass in PASS {
            rows.push(("ui-small", pass, Fixture::ui_small(pass)));
        }
        for pass in PASS {
            rows.push(("ui-large", pass, Fixture::ui_large(pass)));
        }
        for w in WIDTH {
            rows.push(("frame", w, Fixture::frame(w)));
        }
        for (shape, param, f) in rows {
            let (mut marked, mut fired, mut popped) = (0u64, 0u64, 0u64);
            let mut a = f.clone();
            let mut b = f.clone();
            for _ in 0..N {
                let o = a.step(Sched::Mark);
                marked += a.engine.marked() as u64;
                fired += o.fired as u64;
                b.step(Sched::Bucket);
                popped += b.engine.visited() as u64;
            }
            let n = N as f64;
            println!(
                "{:<9} {:>5} {:>6} {:>6} {:>8.1} {:>7.1} {:>7.1} {:>7.1}",
                shape,
                param,
                f.engine.len(),
                f.engine.max_height(),
                marked as f64 / n,
                fired as f64 / n,
                100.0 * (1.0 - fired as f64 / marked as f64),
                popped as f64 / n,
            );
        }
    }
}
