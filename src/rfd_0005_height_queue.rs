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

use crate::rfd_0005_bounded_relink_check::{Checker, Counts, Graph, Id, Move, Run};
use crate::rfd_0005_maintained_rank_queue::{
    Engine, Fixture, HeapQueue, MarkOm, RadixQueue, Schedule, State, Walked,
};

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
