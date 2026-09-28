//! Can RFD 3's collection pause be split across units, and at what cost?
//!
//! RFD 3 ("When Collection Runs") collects between units, never inside
//! one: mark from the roots, prune the dead out of dependents lists, sweep.
//! `rfd_0003_work_paced_trigger` found that a work term in the trigger cuts
//! the cost of garbage and the worst pause, but leaves a floor: in the
//! `app` shape (about ten thousand live nodes) every collection marks all
//! of them, about 1.3 million instructions after one unit, and no trigger
//! removes that. This probe splits it: a tri-colour mark that does a
//! bounded slice after each unit, with a write barrier for every way a unit
//! can change what a black node reaches, then a prune and a sweep that are
//! either atomic or sliced too.
//!
//! # The model
//!
//! The arena, the screens and the `app` workload are
//! `rfd_0005_demand_bounded_push`'s and `rfd_0003_work_paced_trigger`'s,
//! rebuilt here because the colours and the barriers live inside the
//! arena's mutators. The screens are built from the same seeds, so the
//! atomic baseline reproduces that probe's `excess` run (471 collections in
//! 30,000 units, the largest freeing 542). A run is units, every third a
//! navigation (a screen built, the switch moved onto it, `nav` committing
//! its token), the others a click. The trigger is that probe's `excess`:
//! RFD 3's allocation term, or the clicks' regions grown past their
//! reference since the last collection exceed its survivors. For the
//! incremental collector the trigger starts a cycle, and slices then run
//! after each unit until the cycle ends.
//!
//! `Arena<INC>` is generic over whether it has barriers at all, so the
//! atomic baseline, `Arena<false>`, pays nothing for them, and what they
//! cost on the fast path is the difference between the two.
//!
//! # Colours
//!
//! A node is black or grey when its `visit` stamp equals the cycle's epoch
//! (grey while it is on the grey stack), white otherwise. Starting a cycle
//! bumps the epoch, which whitens everything at once, and shades the roots.
//! A mark slice pops grey nodes and shades what they reach: dependencies,
//! recorded reach (a snapshot's cell, `depends`), and the token a cell's
//! committed value holds, validated. New nodes are allocated black in
//! every phase of a cycle, so nothing a unit builds is swept by the cycle
//! it was built in.
//!
//! # What a unit can change mid-cycle, and the barrier for each
//!
//! The barrier is Dijkstra's insertion barrier: when a unit inserts an edge
//! while the mark runs, its target is shaded. Every insertion in Bough goes
//! through the runtime: `construct` allocating a node with its deps and
//! reach, a switch moving onto an inner, a hold committing a value that
//! holds tokens, `depends`, and `listen` or `anchor` adding a root. One
//! deletion doesn't: a guard drops with no runtime access (RFD 3,
//! "Guards"), so no deletion barrier (Yuasa's) can run on it. Yuasa would
//! still cope with guards, since the roots are shaded when the cycle
//! starts, but it isn't enough alone: I/O code can hold a token to a node
//! no root reaches, and anchoring it or building on it mid-cycle adds an
//! edge to a node the snapshot never saw, which Yuasa would free under the
//! new edge. So insertions need a barrier either way, and with one,
//! deletions need none. The cost is that a node reached before its last
//! edge went survives the cycle (floating garbage, freed by the next);
//! Yuasa has the same.
//!
//! - **construct**, [`Arena::alloc`]: the new node is black, so each of its
//!   dependencies and reach targets is shaded. It also joins the prune,
//!   since its dependents list may gain entries.
//! - **switch**, [`Arena::relink`]: the new inner is shaded. The old one
//!   loses an edge and needs nothing.
//! - **hold commit**, [`Arena::commit`]: the committed value's token, if it
//!   validates, is shaded. The old value's token needs nothing.
//! - **depends**, [`Arena::depends`]: each declared target is shaded.
//! - **roots**, [`Arena::guard`]: a new listener's or anchor's node is
//!   shaded. A dropped guard needs nothing, and gets nothing.
//! - **dependents links** change nothing the mark follows (it goes up
//!   dependencies, never down dependents), so they need no mark barrier.
//!   They constrain the prune and the sweep instead: every black node's
//!   list is pruned, those allocated mid-cycle included, and the prune
//!   ends before the sweep frees a slot, since a freed slot is reused and a
//!   stale dependents entry would then reach the new node. Once the mark
//!   ends, white means dead: [`Arena::lookup`] rejects a white node's
//!   token, and transactions skip white dependents, so a dead node that
//!   isn't pruned yet can't run and insert an edge (a dead switch
//!   relinking, a dead `construct` building on its dead inputs).
//!
//! Each barrier is a test of the phase and, while marking, a shade; outside
//! a cycle it is one predictable branch. The tests show each is needed:
//! with it off, a scenario built for it leaves a reachable node white when
//! the mark ends.
//!
//! # Pacing
//!
//! Work is counted in mark-node equivalents, weighted by what each step
//! costs in this probe's instruction counts (about 75 instructions to mark
//! a node, 40 to prune one, 12 to test a slot, 400 to free one; a first cut
//! at `rfd_0003_sweep_cost`'s 60, 30, a few and 205 let sweep slices run
//! long): a mark pop is 16 sixteenths, a pruned node 8, a slot tested 3, a
//! slot freed 80 more. A budget of `k` is `16 k` sixteenths a unit.
//! [`Pace`] is atomic; a fixed `k` a unit, with the prune and sweep atomic
//! (all in the slice where the mark ends) or sliced under the same budget;
//! gc-arena's allocation debt, `c` mark-nodes per node allocated in the
//! unit; or a work debt, `c` per node allocated or per region node past the
//! trigger's reference, the trigger's own measure of garbage work.

use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;

/// A node's index in the arena.
pub type Id = u32;

/// The graph id every token of this model carries.
const GRAPH: u32 = 1;

/// Work weights, in sixteenths of a mark pop.
pub const W_MARK: i64 = 16;
const W_PRUNE: i64 = 8;
const W_SLOT: i64 = 3;
const W_FREE: i64 = 80;

/// A token: index, generation and graph id, as RFD 3 defines it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub index: Id,
    pub generation: u32,
    pub graph: u32,
}

/// What a node computes, boxed in the arena so that freeing a node is a
/// deallocation.
#[derive(Clone, Copy, Debug)]
pub enum Op {
    /// Takes the value sent to it.
    Input,
    /// The sum of its dependencies plus a constant: map, merge, lift.
    Map(i64),
    /// Its one dependency's value: a hold.
    Hold,
    /// Its dependency plus the value of the cell it reads, its reach.
    Snapshot,
    /// Its second dependency's value, the inner its first selects.
    Switch,
}

/// A listener or an anchor as I/O code holds it: a root while it lives.
pub struct Guard(Rc<Cell<bool>>);

impl Drop for Guard {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

/// Where a collection cycle is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Phase {
    /// No cycle: everything allocated counts as live.
    #[default]
    Idle,
    /// Grey nodes remain, so white may still be live.
    Mark,
    /// The mark is done, so white is dead; the black nodes' dependents
    /// lists are being pruned of it.
    Prune,
    /// Pruned; white slots are being freed.
    Sweep,
}

/// The barriers, so a test can turn one off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Barrier {
    Construct,
    Switch,
    Hold,
    Depends,
    Root,
}

/// What a cycle did, reported when it ends.
#[derive(Clone, Copy, Debug, Default)]
pub struct Cycle {
    pub freed: usize,
    pub survivors: usize,
    /// Nodes a barrier shaded that were white.
    pub shaded: usize,
    /// Nodes allocated black during the cycle.
    pub born: usize,
}

/// What one unit's collection work did.
#[derive(Clone, Copy, Debug, Default)]
pub struct Slice {
    pub marked: usize,
    pub pruned: usize,
    pub slots: usize,
    pub freed: usize,
    /// The cycle, if this slice ended one.
    pub ended: Option<Cycle>,
}

/// The arena, its roots, the collector's state, and the scratch
/// transactions reuse. `INC` compiles the barriers and the phase tests in.
#[derive(Clone, Default)]
pub struct Arena<const INC: bool> {
    generation: Vec<u32>,
    live: Vec<bool>,
    /// The epoch of the cycle that last shaded the node, or of its birth.
    visit: Vec<u32>,
    /// The transaction's mark stamp.
    mark: Vec<u32>,
    deps: Vec<Vec<Id>>,
    /// Marking a transaction follows these.
    dependents: Vec<Vec<Id>>,
    /// Reach that is not a dependency: a snapshot's cell, `depends`.
    reach: Vec<Vec<Id>>,
    /// The token a cell's committed value holds, which the mark traces.
    held: Vec<Option<Token>>,
    op: Vec<Option<Box<Op>>>,
    value: Vec<i64>,
    /// The allocation serial: which node this is, comparable across runs
    /// whose slots were reused in different orders.
    serial: Vec<u64>,
    /// Freed slots, oldest first.
    free: VecDeque<Id>,
    live_count: usize,
    allocated: u64,
    roots: Vec<(Id, Rc<Cell<bool>>)>,
    epoch: u32,
    tx: u32,
    phase: Phase,
    gray: Vec<Id>,
    /// The black nodes whose dependents lists the prune walks.
    reached: Vec<Id>,
    /// The prune's index into `reached`, or the sweep's slot.
    cursor: usize,
    cycle: Cycle,
    order: Vec<Id>,
    stack: Vec<(Id, usize)>,
    #[cfg(test)]
    off: Option<Barrier>,
}

impl<const INC: bool> Arena<INC> {
    /// Slots in the arena, live or free.
    pub fn slots(&self) -> usize {
        self.live.len()
    }

    /// Nodes allocated and not yet freed, dead or alive.
    pub fn live_count(&self) -> usize {
        self.live_count
    }

    /// Nodes allocated since the arena was made.
    pub fn allocated(&self) -> u64 {
        self.allocated
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The last transaction's region, in evaluation order reversed.
    pub fn region(&self) -> &[Id] {
        &self.order
    }

    pub fn value(&self, i: Id) -> i64 {
        self.value[i as usize]
    }

    fn is_black(&self, i: Id) -> bool {
        self.visit[i as usize] == self.epoch
    }

    /// Whether barrier `b` runs: while marking, unless a test turned it off.
    #[inline]
    fn barrier(&self, _b: Barrier) -> bool {
        #[cfg(test)]
        if self.off == Some(_b) {
            return false;
        }
        INC && self.phase == Phase::Mark
    }

    /// Shades `i` grey if it is white: a barrier's slow path.
    #[inline]
    fn shade(&mut self, i: Id) {
        let v = &mut self.visit[i as usize];
        if *v != self.epoch {
            *v = self.epoch;
            self.gray.push(i);
            self.cycle.shaded += 1;
        }
    }

    /// Allocates a node black (stamped with the current epoch), so a node
    /// born mid-cycle survives it. Its edges are insertions, so while the
    /// mark runs their targets are shaded (the construct barrier), and the
    /// node joins the prune.
    pub fn alloc(&mut self, op: Op, deps: &[Id], reach: &[Id]) -> Id {
        let i = match self.free.pop_front() {
            Some(i) => {
                let n = i as usize;
                self.live[n] = true;
                self.deps[n].clear();
                self.dependents[n].clear();
                self.reach[n].clear();
                self.op[n] = Some(Box::new(op));
                self.value[n] = 0;
                i
            }
            None => {
                let i = Id::try_from(self.live.len()).expect("fewer than u32::MAX nodes");
                self.generation.push(0);
                self.live.push(true);
                self.visit.push(0);
                self.mark.push(0);
                self.deps.push(Vec::new());
                self.dependents.push(Vec::new());
                self.reach.push(Vec::new());
                self.held.push(None);
                self.op.push(Some(Box::new(op)));
                self.value.push(0);
                self.serial.push(0);
                i
            }
        };
        let n = i as usize;
        self.visit[n] = self.epoch;
        self.serial[n] = self.allocated;
        self.live_count += 1;
        self.allocated += 1;
        self.deps[n].extend_from_slice(deps);
        self.reach[n].extend_from_slice(reach);
        for &d in deps {
            self.dependents[d as usize].push(i);
        }
        if INC && self.phase != Phase::Idle {
            self.cycle.born += 1;
            if self.phase == Phase::Mark {
                self.reached.push(i);
            }
            if self.barrier(Barrier::Construct) {
                for &d in deps.iter().chain(reach) {
                    self.shade(d);
                }
            }
        }
        i
    }

    pub fn token(&self, i: Id) -> Token {
        Token {
            index: i,
            generation: self.generation[i as usize],
            graph: GRAPH,
        }
    }

    fn valid(&self, t: Token) -> bool {
        let n = t.index as usize;
        t.graph == GRAPH
            && n < self.live.len()
            && self.live[n]
            && self.generation[n] == t.generation
    }

    /// Validates a token. Once a cycle's mark has ended, a white node is
    /// dead though not yet freed, and its token fails as a freed one's does.
    pub fn lookup(&self, t: Token) -> Option<Id> {
        let dead =
            INC && matches!(self.phase, Phase::Prune | Phase::Sweep) && !self.is_black(t.index);
        (self.valid(t) && !dead).then_some(t.index)
    }

    /// Commits a value holding `t`, or no token, to cell `i`. The new token
    /// is an insertion (the hold barrier); losing the old one needs nothing.
    pub fn commit(&mut self, i: Id, t: Option<Token>) {
        self.held[i as usize] = t;
        if let Some(t) = t
            && self.barrier(Barrier::Hold)
            && self.valid(t)
        {
            self.shade(t.index);
        }
    }

    /// `depends`: node `i` keeps `targets` alive from now on.
    pub fn depends(&mut self, i: Id, targets: &[Id]) {
        self.reach[i as usize].extend_from_slice(targets);
        if self.barrier(Barrier::Depends) {
            for &d in targets {
                self.shade(d);
            }
        }
    }

    /// Roots node `i` for as long as the returned guard lives (the root
    /// barrier). Dropping the guard touches only its flag.
    pub fn guard(&mut self, i: Id) -> Guard {
        let flag = Rc::new(Cell::new(true));
        self.roots.push((i, flag.clone()));
        if self.barrier(Barrier::Root) {
            self.shade(i);
        }
        Guard(flag)
    }

    /// Moves switch `sw`, whose first dependency is its outer, from inner
    /// `old` (if any) to inner `new`: RFD 5's relink, with the switch
    /// barrier on the new inner.
    pub fn relink(&mut self, sw: Id, old: Option<Id>, new: Id) {
        if let Some(old) = old {
            self.dependents[old as usize].retain(|&d| d != sw);
        }
        let deps = &mut self.deps[sw as usize];
        deps.truncate(1);
        deps.push(new);
        self.dependents[new as usize].push(sw);
        if self.barrier(Barrier::Switch) {
            self.shade(new);
        }
    }

    /// Grows the scratch to what the arena could need, as a runtime past its
    /// first collection has it. `Vec::clone` drops spare capacity, so a
    /// clone calls this again.
    pub fn reserve_scratch(&mut self) {
        let n = self.slots();
        self.gray.reserve(n);
        self.reached.reserve(n);
        self.order.reserve(n);
        self.stack.reserve(n);
        self.free.reserve(n);
    }

    /// Starts a cycle: bumps the epoch, which whitens every node, forgets
    /// the released roots, and shades the live ones.
    pub fn start_cycle(&mut self) {
        debug_assert_eq!(self.phase, Phase::Idle);
        self.epoch += 1;
        self.phase = Phase::Mark;
        self.cycle = Cycle::default();
        self.reached.clear();
        self.gray.clear();
        self.roots.retain(|(_, flag)| flag.get());
        let epoch = self.epoch;
        for &(i, _) in &self.roots {
            let v = &mut self.visit[i as usize];
            if *v != epoch {
                *v = epoch;
                self.gray.push(i);
            }
        }
    }

    /// Runs the cycle for up to `budget` sixteenths of work: the prune and
    /// the sweep under the same budget if `sliced`, else all at once in the
    /// slice where the mark ends. A budget of `i64::MAX` ends the cycle.
    pub fn slice(&mut self, budget: i64, sliced: bool) -> Slice {
        let mut s = Slice::default();
        let mut left = budget;
        if self.phase == Phase::Mark {
            left = self.mark_slice(left, &mut s);
            if self.gray.is_empty() {
                self.phase = Phase::Prune;
                self.cursor = 0;
            }
        }
        if !sliced && self.phase != Phase::Mark {
            left = i64::MAX;
        }
        if self.phase == Phase::Prune && left > 0 {
            left = self.prune_slice(left, &mut s);
            if self.cursor == self.reached.len() {
                self.phase = Phase::Sweep;
                self.cursor = 0;
            }
        }
        if self.phase == Phase::Sweep && left > 0 {
            self.sweep_slice(left, &mut s);
            if self.cursor == self.live.len() {
                self.phase = Phase::Idle;
                self.cycle.survivors = self.live_count;
                s.ended = Some(self.cycle);
            }
        }
        s
    }

    /// A whole collection: ends the running cycle, or runs one.
    pub fn collect(&mut self) -> Cycle {
        if self.phase == Phase::Idle {
            self.start_cycle();
        }
        self.slice(i64::MAX, false)
            .ended
            .expect("an unbounded slice ends the cycle")
    }

    fn mark_slice(&mut self, mut left: i64, s: &mut Slice) -> i64 {
        let epoch = self.epoch;
        let Arena {
            generation,
            live,
            visit,
            deps,
            reach,
            held,
            gray,
            reached,
            ..
        } = self;
        let shade = |visit: &mut Vec<u32>, gray: &mut Vec<Id>, i: Id| {
            let v = &mut visit[i as usize];
            if *v != epoch {
                *v = epoch;
                gray.push(i);
            }
        };
        while left > 0 {
            let Some(n) = gray.pop() else { break };
            reached.push(n);
            let at = n as usize;
            for &d in &deps[at] {
                shade(visit, gray, d);
            }
            for &r in &reach[at] {
                shade(visit, gray, r);
            }
            // A token that fails its check names nothing.
            if let Some(t) = held[at] {
                let m = t.index as usize;
                if t.graph == GRAPH && m < live.len() && live[m] && generation[m] == t.generation {
                    shade(visit, gray, t.index);
                }
            }
            s.marked += 1;
            left -= W_MARK;
        }
        left
    }

    /// Takes the white out of the black nodes' dependents lists, keeping
    /// the order of the rest.
    fn prune_slice(&mut self, mut left: i64, s: &mut Slice) -> i64 {
        let Arena {
            visit,
            dependents,
            reached,
            epoch,
            cursor,
            ..
        } = self;
        while left > 0 && *cursor < reached.len() {
            let n = reached[*cursor] as usize;
            dependents[n].retain(|&d| visit[d as usize] == *epoch);
            *cursor += 1;
            s.pruned += 1;
            left -= W_PRUNE;
        }
        left
    }

    /// Frees white slots from the cursor on, in index order: drops the
    /// payload and held token, bumps the generation, and puts the slot on
    /// the back of the free list.
    fn sweep_slice(&mut self, mut left: i64, s: &mut Slice) {
        let epoch = self.epoch;
        let end = self.live.len();
        let (start, mut freed) = (self.cursor, 0);
        while left > 0 && self.cursor < end {
            let n = self.cursor;
            self.cursor += 1;
            left -= W_SLOT;
            if self.live[n] && self.visit[n] != epoch {
                self.live[n] = false;
                self.op[n] = None;
                self.held[n] = None;
                self.generation[n] += 1;
                self.free.push_back(n as Id);
                freed += 1;
                left -= W_FREE;
            }
        }
        self.live_count -= freed;
        self.cycle.freed += freed;
        s.freed += freed;
        s.slots += self.cursor - start;
    }

    /// One transaction: sends `x` to `input`, marks what depends on it over
    /// dependents, and evaluates the reverse post-order. Once a cycle's
    /// mark has ended, white dependents are dead and are skipped.
    pub fn transaction(&mut self, input: Id, x: i64) {
        if INC && matches!(self.phase, Phase::Prune | Phase::Sweep) {
            self.mark_region::<true>(input);
        } else {
            self.mark_region::<false>(input);
        }
        self.evaluate(x);
    }

    fn mark_region<const SKIP: bool>(&mut self, input: Id) {
        self.tx += 1;
        let tx = self.tx;
        let epoch = self.epoch;
        let Arena {
            visit,
            mark,
            dependents,
            order,
            stack,
            ..
        } = self;
        order.clear();
        stack.clear();
        mark[input as usize] = tx;
        stack.push((input, 0));
        while let Some(&mut (n, ref mut k)) = stack.last_mut() {
            let ds = &dependents[n as usize];
            if *k < ds.len() {
                let d = ds[*k];
                *k += 1;
                let m = &mut mark[d as usize];
                if *m != tx && (!SKIP || visit[d as usize] == epoch) {
                    *m = tx;
                    stack.push((d, 0));
                }
            } else {
                order.push(n);
                stack.pop();
            }
        }
    }

    fn evaluate(&mut self, x: i64) {
        for k in (0..self.order.len()).rev() {
            let n = self.order[k] as usize;
            let op = **self.op[n].as_ref().expect("a marked node is allocated");
            let v = match op {
                Op::Input => x,
                Op::Map(c) => self.deps[n]
                    .iter()
                    .fold(c, |a, &d| a.wrapping_add(self.value[d as usize])),
                Op::Hold => self.value[self.deps[n][0] as usize],
                Op::Snapshot => self.value[self.deps[n][0] as usize]
                    .wrapping_add(self.value[self.reach[n][0] as usize]),
                Op::Switch => self.value[self.deps[n][1] as usize],
            };
            self.value[n] = v;
        }
    }

    /// An independent walk from the live roots over the edges the mark
    /// follows: which slots are reachable now. The tests' oracle.
    pub fn reachable(&self) -> Vec<bool> {
        let mut seen = vec![false; self.slots()];
        let mut stack: Vec<Id> = self
            .roots
            .iter()
            .filter(|(_, f)| f.get())
            .map(|&(i, _)| i)
            .collect();
        while let Some(n) = stack.pop() {
            let at = n as usize;
            if std::mem::replace(&mut seen[at], true) {
                continue;
            }
            assert!(self.live[at], "reachable node {n} was freed");
            stack.extend(&self.deps[at]);
            stack.extend(&self.reach[at]);
            if let Some(t) = self.held[at]
                && self.valid(t)
            {
                stack.push(t.index);
            }
        }
        seen
    }

    /// Reachable nodes the mark left white. When the mark has just ended
    /// this must be empty, since white is about to be freed.
    pub fn white_but_reachable(&self) -> Vec<Id> {
        self.reachable()
            .iter()
            .enumerate()
            .filter(|&(n, &r)| r && !self.is_black(n as Id))
            .map(|(n, _)| n as Id)
            .collect()
    }

    /// The allocated nodes' serials, sorted.
    pub fn live_serials(&self) -> Vec<u64> {
        let mut v: Vec<u64> = (0..self.slots())
            .filter(|&n| self.live[n])
            .map(|n| self.serial[n])
            .collect();
        v.sort_unstable();
        v
    }

    /// The reachable nodes' serials, sorted.
    pub fn reachable_serials(&self) -> Vec<u64> {
        let mut v: Vec<u64> = self
            .reachable()
            .iter()
            .enumerate()
            .filter(|&(_, &r)| r)
            .map(|(n, _)| self.serial[n])
            .collect();
        v.sort_unstable();
        v
    }
}

/// SplitMix64 with fixed seeds, so every run builds the same graph.
#[derive(Clone)]
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn range(&mut self, lo: u32, hi: u32) -> u32 {
        lo + (self.next() % u64::from(hi - lo + 1)) as u32
    }
}

/// `rfd_0005_demand_bounded_push`'s screen, 18 to 30 nodes: branches of
/// maps off `src` (the first also reading the theme, the second `side`),
/// merged, held, snapshotted, and a cell holding the token of a node that
/// depends on it, a cycle through values. Returns its last node.
fn screen<const INC: bool>(a: &mut Arena<INC>, rng: &mut Rng, src: Id, side: Id, theme: Id) -> Id {
    let mut ends = Vec::new();
    for b in 0..rng.range(3, 4) {
        let mut at = match b {
            0 => a.alloc(Op::Map(1), &[src, theme], &[]),
            1 => a.alloc(Op::Map(2), &[src, side], &[]),
            _ => a.alloc(Op::Map(i64::from(b)), &[src], &[]),
        };
        for c in 0..rng.range(3, 5) {
            at = a.alloc(Op::Map(i64::from(c)), &[at], &[]);
        }
        ends.push(at);
    }
    let merge = a.alloc(Op::Map(0), &ends, &[]);
    let count = a.alloc(Op::Hold, &[merge], &[]);
    let snap = a.alloc(Op::Snapshot, &[src], &[count]);
    let select = a.alloc(Op::Hold, &[snap], &[]);
    let detail = a.alloc(Op::Map(0), &[select, theme], &[]);
    let t = a.token(detail);
    a.commit(select, Some(t));
    a.alloc(Op::Map(0), &[count, select], &[])
}

/// Screens kept live in the `app` shape, as in the work-paced probe.
pub const APP_KEPT: usize = 430;

/// The measured window: 1,300 navigations and 2,600 clicks.
pub const WINDOW: usize = 3_900;

/// How the collector is paced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pace {
    /// The whole collection, after the unit whose trigger fired.
    Atomic,
    /// `k` mark-nodes of work a unit; the prune and the sweep sliced under
    /// the same budget, or atomic in the slice where the mark ends.
    Fixed { k: i64, sliced: bool },
    /// gc-arena's debt: `c` mark-nodes per node allocated in the unit.
    AllocDebt { c: i64 },
    /// `c` mark-nodes per node allocated, or per region node past the
    /// trigger's reference, in the unit.
    WorkDebt { c: i64 },
}

impl Pace {
    pub fn name(self) -> String {
        match self {
            Pace::Atomic => "atomic".into(),
            Pace::Fixed { k, sliced: false } => format!("k{k}-atomic-sweep"),
            Pace::Fixed { k, sliced: true } => format!("k{k}"),
            Pace::AllocDebt { c } => format!("alloc-debt{c}"),
            Pace::WorkDebt { c } => format!("work-debt{c}"),
        }
    }

    fn sliced(self) -> bool {
        match self {
            Pace::Atomic => false,
            Pace::Fixed { sliced, .. } => sliced,
            Pace::AllocDebt { .. } | Pace::WorkDebt { .. } => true,
        }
    }
}

/// The incremental paces the benches and the counts run.
pub const PACES: [Pace; 8] = [
    Pace::Fixed {
        k: 250,
        sliced: true,
    },
    Pace::Fixed {
        k: 1_000,
        sliced: true,
    },
    Pace::Fixed {
        k: 4_000,
        sliced: true,
    },
    Pace::Fixed {
        k: 250,
        sliced: false,
    },
    Pace::Fixed {
        k: 1_000,
        sliced: false,
    },
    Pace::Fixed {
        k: 4_000,
        sliced: false,
    },
    Pace::AllocDebt { c: 2 },
    Pace::WorkDebt { c: 2 },
];

/// The work-paced probe's `excess` trigger: RFD 3's allocation term, or
/// the clicks' regions grown past their reference since the last
/// collection exceed its survivors. One input fires, so one reference.
#[derive(Clone, Debug)]
struct Trigger {
    survivors: usize,
    work: usize,
    /// The region of the first click after the last collection.
    reference: Option<usize>,
}

impl Trigger {
    /// Returns the region's growth past the reference: garbage work.
    #[inline]
    fn observe(&mut self, region: usize) -> usize {
        let at = *self.reference.get_or_insert(region);
        let excess = region.saturating_sub(at);
        self.work += excess;
        excess
    }

    #[inline]
    fn due(&self, live: usize) -> bool {
        live.saturating_sub(self.survivors) > self.survivors || self.work > self.survivors
    }

    fn collected(&mut self, survivors: usize) {
        self.survivors = survivors;
        self.work = 0;
        self.reference = None;
    }
}

/// One unit in the model's terms, for finding the costliest.
#[derive(Clone, Copy, Debug, Default)]
pub struct UnitCost {
    /// Its index from the window's start.
    pub unit: usize,
    pub region: usize,
    pub allocated: usize,
    pub slice: Slice,
}

impl UnitCost {
    /// Estimated instructions, fitted to this probe's own instruction
    /// counts (a first cut at `rfd_0003_sweep_cost`'s figures picked the
    /// wrong units): about 100 a region node, 850 a node built, 75 a node
    /// marked, 40 pruned, 12 a slot tested and 400 a slot freed.
    pub fn estimate(&self) -> usize {
        let s = &self.slice;
        100 * self.region
            + 850 * self.allocated
            + 75 * s.marked
            + 40 * s.pruned
            + 12 * s.slots
            + 400 * s.freed
    }
}

/// What a run has done since its window started.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub units: usize,
    pub clicks: usize,
    /// Region nodes over every click.
    pub region_nodes: usize,
    /// Cycles ended.
    pub cycles: usize,
    pub freed: usize,
    /// Units that did collection work, and the longest cycle in units.
    pub collecting_units: usize,
    pub longest_cycle: usize,
    /// Nodes marked and pruned, and slots swept, over every slice.
    pub marked: usize,
    pub pruned: usize,
    pub slots: usize,
    /// White nodes barriers shaded, and nodes born black, over every cycle.
    pub shaded: usize,
    pub born: usize,
    pub largest_freed: usize,
    pub max_slots: usize,
    /// The unit the model rates costliest.
    pub worst: UnitCost,
    /// The sum of the switch's values, which every pace must agree on.
    pub checksum: i64,
}

/// The `app` workload under one pace: F66's navigation graph with 430
/// screens kept live on the clock beside it.
pub struct Run<const INC: bool> {
    pub arena: Arena<INC>,
    pub pace: Pace,
    clicks: Id,
    clock: Id,
    theme: Id,
    nav: Id,
    switch: Id,
    current: Option<Id>,
    rng: Rng,
    guards: Rc<Vec<Guard>>,
    trigger: Trigger,
    /// Units since the run began, which sets the schedule.
    unit: usize,
    /// Units the running cycle has had.
    in_cycle: usize,
    /// Checks at every mark's end that no reachable node is white.
    pub audit: bool,
    pub stats: Stats,
}

impl<const INC: bool> Clone for Run<INC> {
    fn clone(&self) -> Self {
        let mut arena = self.arena.clone();
        arena.reserve_scratch();
        Run {
            arena,
            rng: self.rng.clone(),
            guards: self.guards.clone(),
            trigger: self.trigger.clone(),
            stats: self.stats.clone(),
            ..*self
        }
    }
}

impl<const INC: bool> Run<INC> {
    /// The core, `kept` screens kept live by anchors, one screen on show,
    /// collected, one click run: `rfd_0005_demand_bounded_push`'s
    /// `Fixture::new(kept, 0, Collect).warmed(All)`, seed for seed.
    pub fn new(kept: usize, pace: Pace) -> Self {
        assert!(INC || pace == Pace::Atomic, "no barriers, no increments");
        let mut a = Arena::<INC>::default();
        let mut guards = Vec::new();
        // The core the I/O code anchors.
        let clicks = a.alloc(Op::Input, &[], &[]);
        let clock = a.alloc(Op::Input, &[], &[]);
        let navigate = a.alloc(Op::Input, &[], &[]);
        let theme = a.alloc(Op::Hold, &[clock], &[]);
        let nav = a.alloc(Op::Hold, &[navigate], &[]);
        let switch = a.alloc(Op::Switch, &[nav], &[]);
        for i in [clicks, clock, navigate, switch] {
            guards.push(a.guard(i));
        }
        let mut rng = Rng(0x5EED_0005);
        for _ in 0..kept {
            let last = screen(&mut a, &mut rng, clock, theme, theme);
            guards.push(a.guard(last));
        }
        let mut r = Run {
            arena: a,
            pace,
            clicks,
            clock,
            theme,
            nav,
            switch,
            current: None,
            rng,
            guards: Rc::new(guards),
            trigger: Trigger {
                survivors: 0,
                work: 0,
                reference: None,
            },
            unit: 0,
            in_cycle: 0,
            audit: false,
            stats: Stats::default(),
        };
        r.navigate_to(&mut Rng(0xBA5E));
        r.arena.collect();
        r.arena.reserve_scratch();
        r.arena.transaction(r.clicks, 0);
        r.trigger.survivors = r.arena.live_count();
        r
    }

    /// Runs units until the first cycle has ended, so that the window
    /// starts where the pace's own cycle does, then starts the window.
    pub fn warmed(mut self) -> Self {
        while self.stats.cycles == 0 {
            self.unit();
        }
        self.stats = Stats::default();
        self.arena.reserve_scratch();
        self
    }

    /// A navigation, to a screen of random shape.
    pub fn navigate(&mut self) {
        let mut rng = self.rng.clone();
        self.navigate_to(&mut rng);
        self.rng = rng;
    }

    /// A construct builds a screen, the switch moves onto it, and `nav`
    /// commits its token, which leaves the old screen unreachable.
    fn navigate_to(&mut self, rng: &mut Rng) {
        let last = screen(&mut self.arena, rng, self.clicks, self.clock, self.theme);
        self.arena.relink(self.switch, self.current, last);
        let t = self.arena.token(last);
        self.arena.commit(self.nav, Some(t));
        self.current = Some(last);
    }

    /// One unit's work, without collection: a navigation every third unit,
    /// else a click. Returns the click's region, the nodes built, and the
    /// region's growth past the trigger's reference.
    pub fn step(&mut self) -> (usize, usize, usize) {
        let k = self.unit;
        self.unit += 1;
        self.stats.units += 1;
        if k.is_multiple_of(3) {
            let before = self.arena.allocated();
            self.navigate();
            return (0, (self.arena.allocated() - before) as usize, 0);
        }
        self.arena.transaction(self.clicks, k as i64);
        let region = self.arena.region().len();
        let excess = self.trigger.observe(region);
        let s = &mut self.stats;
        s.clicks += 1;
        s.region_nodes += region;
        s.checksum = s.checksum.wrapping_add(self.arena.value(self.switch));
        (region, 0, excess)
    }

    /// After a unit: starts a cycle if none is running and the trigger is
    /// due, then runs this unit's slice of the one running.
    pub fn settle(&mut self, allocated: usize, excess: usize) -> Slice {
        self.stats.max_slots = self.stats.max_slots.max(self.arena.slots());
        if self.arena.phase() == Phase::Idle {
            if !self.trigger.due(self.arena.live_count()) {
                return Slice::default();
            }
            self.arena.start_cycle();
            self.in_cycle = 0;
        }
        let budget = match self.pace {
            Pace::Atomic => i64::MAX,
            Pace::Fixed { k, .. } => k * W_MARK,
            Pace::AllocDebt { c } => c * allocated as i64 * W_MARK,
            Pace::WorkDebt { c } => c * (allocated + excess) as i64 * W_MARK,
        };
        let was_marking = self.arena.phase() == Phase::Mark;
        let s = self.arena.slice(budget, self.pace.sliced());
        if self.audit && was_marking && self.arena.phase() != Phase::Mark {
            let lost = self.arena.white_but_reachable();
            assert!(
                lost.is_empty(),
                "reachable but white at the mark's end: {lost:?}"
            );
        }
        self.in_cycle += 1;
        let st = &mut self.stats;
        st.collecting_units += 1;
        st.marked += s.marked;
        st.pruned += s.pruned;
        st.slots += s.slots;
        if let Some(c) = s.ended {
            self.trigger.collected(c.survivors);
            st.cycles += 1;
            st.freed += c.freed;
            st.shaded += c.shaded;
            st.born += c.born;
            st.largest_freed = st.largest_freed.max(c.freed);
            st.longest_cycle = st.longest_cycle.max(self.in_cycle);
        }
        s
    }

    /// One whole unit, collection work included.
    pub fn unit(&mut self) -> UnitCost {
        let (region, allocated, excess) = self.step();
        let slice = self.settle(allocated, excess);
        let cost = UnitCost {
            unit: self.stats.units - 1,
            region,
            allocated,
            slice,
        };
        if cost.estimate() > self.stats.worst.estimate() {
            self.stats.worst = cost;
        }
        cost
    }

    /// Runs `units` units. Returns the checksum.
    pub fn run(&mut self, units: usize) -> i64 {
        for _ in 0..units {
            self.unit();
        }
        self.stats.checksum
    }

    /// Ends the running cycle, if any, then runs one more whole one.
    pub fn drain(&mut self) {
        if self.arena.phase() != Phase::Idle {
            self.arena.collect();
        }
        self.arena.collect();
    }

    /// A warmed run's window replayed up to the unit the model rates
    /// costliest, stopped right before it: `unit()` on it is the pace's
    /// worst per-unit pause.
    pub fn before_worst(kept: usize, pace: Pace, units: usize) -> Self {
        let start = Run::new(kept, pace).warmed();
        let mut probe = start.clone();
        probe.run(units);
        let at = probe.stats.worst.unit;
        let mut run = start;
        while run.stats.units < at {
            run.unit();
        }
        run.arena.reserve_scratch();
        run
    }

    /// The fast path: a freshly collected `app` arena with no garbage,
    /// whose next unit is a click, or a navigation if `navigate`, with a
    /// cycle in `phase` (for `Mark`, started and one slice of 1,000 run;
    /// for `Prune` and `Sweep`, marked). `step()` on it is one unit with
    /// the barriers in that state and no collection work.
    pub fn fast_path(kept: usize, phase: Phase, navigate: bool) -> Self {
        let pace = if INC {
            Pace::Fixed {
                k: 1_000,
                sliced: true,
            }
        } else {
            Pace::Atomic
        };
        let mut r = Run::new(kept, pace);
        r.unit = if navigate { 3 } else { 1 };
        if phase != Phase::Idle {
            r.arena.start_cycle();
            r.arena.slice(1_000 * W_MARK, true);
            while r.arena.phase() != phase {
                r.arena.slice(W_MARK, true);
            }
        }
        assert_eq!(r.arena.phase(), phase);
        r.arena.reserve_scratch();
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_atomic() -> impl Iterator<Item = Pace> {
        [Pace::Atomic].into_iter().chain(PACES)
    }

    /// The atomic baseline is the work-paced probe's `excess` run: the
    /// same collections over the same 30,000 units.
    #[test]
    fn the_baseline_reproduces_the_work_paced_probe() {
        let mut r = Run::<false>::new(APP_KEPT, Pace::Atomic).warmed();
        r.run(30_000);
        assert_eq!(r.stats.cycles, 471);
        assert_eq!(r.stats.largest_freed, 542);
    }

    /// Every pace computes the switch values the atomic baseline does; at
    /// every mark's end no reachable node is white; and after the running
    /// cycle and one more, exactly the reachable nodes are alive, the same
    /// ones the atomic collector leaves.
    #[test]
    fn incremental_frees_exactly_what_atomic_frees() {
        const UNITS: usize = 3_000;
        let mut atomic = Run::<false>::new(APP_KEPT, Pace::Atomic);
        atomic.audit = true;
        let sum = atomic.run(UNITS);
        atomic.drain();
        let alive = atomic.arena.live_serials();
        assert_eq!(alive, atomic.arena.reachable_serials());
        for pace in with_atomic() {
            let mut r = Run::<true>::new(APP_KEPT, pace);
            r.audit = true;
            assert_eq!(r.run(UNITS), sum, "{pace:?}: values");
            assert!(r.stats.cycles > 0, "{pace:?}: no cycle ended");
            r.drain();
            assert_eq!(r.arena.live_serials(), alive, "{pace:?}: live set");
            eprintln!(
                "{}: {} cycles, {} freed; after draining {} live, as atomic",
                pace.name(),
                r.stats.cycles,
                r.stats.freed,
                alive.len()
            );
        }
    }

    /// A small graph for the barrier scenarios: `c` is reached only through
    /// `b`'s held token, `a` and `b` are rooted, and `a` is popped first.
    struct Scene {
        arena: Arena<true>,
        a: Id,
        b: Id,
        c: Id,
        guards: Vec<Guard>,
    }

    fn scene(a_op: Op) -> Scene {
        let mut arena = Arena::<true>::default();
        let outer = arena.alloc(Op::Input, &[], &[]);
        let c = arena.alloc(Op::Input, &[], &[]);
        let b = arena.alloc(Op::Hold, &[], &[]);
        let t = arena.token(c);
        arena.commit(b, Some(t));
        let a = arena.alloc(a_op, &[outer], &[]);
        let guards = vec![arena.guard(b), arena.guard(a)];
        Scene {
            arena,
            a,
            b,
            c,
            guards,
        }
    }

    /// Runs `mutate` once `a` is black and `b` still grey, then commits a
    /// value with no token to `b`, deleting its edge to `c`, and finishes
    /// the mark. Returns the reachable nodes it left white.
    fn scenario(off: Option<Barrier>, a_op: Op, mutate: fn(&mut Scene)) -> Vec<Id> {
        let mut s = scene(a_op);
        s.arena.off = off;
        s.arena.start_cycle();
        s.arena.slice(W_MARK, true);
        assert!(s.arena.is_black(s.a) && !s.arena.is_black(s.c));
        mutate(&mut s);
        s.arena.commit(s.b, None);
        while s.arena.phase() == Phase::Mark {
            s.arena.slice(W_MARK, true);
        }
        s.arena.white_but_reachable()
    }

    /// Each barrier is needed: the scenario built for it leaves a reachable
    /// node white, about to be freed, with that barrier off, and none with
    /// every barrier on.
    #[test]
    fn every_barrier_is_needed() {
        type Mutation = fn(&mut Scene);
        let cases: [(Barrier, Op, Mutation); 5] = [
            // A hold commits a value holding `c`'s token.
            (Barrier::Hold, Op::Hold, |s| {
                let t = s.arena.token(s.c);
                s.arena.commit(s.a, Some(t));
            }),
            // A switch moves onto `c`.
            (Barrier::Switch, Op::Switch, |s| {
                s.arena.relink(s.a, None, s.c)
            }),
            // `a` declares it keeps `c`.
            (Barrier::Depends, Op::Map(0), |s| {
                s.arena.depends(s.a, &[s.c])
            }),
            // A construct builds an anchored node over `c`.
            (Barrier::Construct, Op::Map(0), |s| {
                let n = s.arena.alloc(Op::Map(0), &[s.c], &[]);
                let g = s.arena.guard(n);
                s.guards.push(g);
            }),
            // I/O code anchors `c`.
            (Barrier::Root, Op::Map(0), |s| {
                let g = s.arena.guard(s.c);
                s.guards.push(g);
            }),
        ];
        for (b, op, mutate) in cases {
            let on = scenario(None, op, mutate);
            assert!(on.is_empty(), "{b:?} on: {on:?} left white");
            let off = scenario(Some(b), op, mutate);
            assert!(!off.is_empty(), "{b:?} off: nothing lost");
        }
    }

    /// Once the mark has ended, a white node's token fails, and a
    /// transaction skips it.
    #[test]
    fn white_is_dead_after_the_mark() {
        let mut r = Run::<true>::new(APP_KEPT, PACES[0]);
        let old = r.arena.token(r.current.unwrap());
        r.navigate();
        r.arena.start_cycle();
        assert!(
            r.arena.lookup(old).is_some(),
            "white may be live while marking"
        );
        while r.arena.phase() == Phase::Mark {
            r.arena.slice(1_000 * W_MARK, true);
        }
        assert_eq!(r.arena.lookup(old), None);
        r.arena.transaction(r.clicks, 1);
        assert!(r.arena.region().iter().all(|&n| r.arena.is_black(n)));
    }

    /// The counts the note quotes: per pace over 30,000 units, cycles and
    /// their length in units, the clicks' regions, collection work per
    /// unit, and the unit the model rates costliest in the benches' window.
    #[test]
    fn counts() {
        const UNITS: usize = 30_000;
        let base = Run::<false>::new(APP_KEPT, Pace::Atomic).warmed();
        println!(
            "app: {} live after a collection; {UNITS} units from each pace's first cycle, every \
             third a navigation, the rest a click; trigger: the work-paced probe's `excess`",
            base.arena.live_count()
        );
        println!(
            "budget k = k mark-node equivalents a unit (mark 1, prune 1/2, slot tested 3/16, slot \
             freed +5); `-atomic-sweep`: prune and sweep all in the slice where the mark ends"
        );
        println!(
            "columns: u/cyc, maxcyc = units a cycle runs, mean and longest; rgn/clk = region \
             nodes a click; coll/u = collection work a unit in mark-node equivalents; shaded, \
             born = white nodes barriers shaded, nodes born black, a cycle"
        );
        println!(
            "  {:<18} {:>6} {:>6} {:>6} {:>8} {:>7} {:>8} {:>8} {:>7} {:>6} {:>6}",
            "pace",
            "cycles",
            "u/cyc",
            "maxcyc",
            "rgn/clk",
            "coll/u",
            "freed/c",
            "largest",
            "shaded",
            "born",
            "slots"
        );
        let row = |name: &str, s: &Stats| {
            let work = s.marked as f64
                + s.pruned as f64 / 2.0
                + s.slots as f64 * 3.0 / 16.0
                + s.freed as f64 * 5.0;
            let c = s.cycles.max(1) as f64;
            println!(
                "  {:<18} {:>6} {:>6.1} {:>6} {:>8.1} {:>7.1} {:>8.1} {:>8} {:>7.1} {:>6.1} {:>6}",
                name,
                s.cycles,
                s.collecting_units as f64 / c,
                s.longest_cycle,
                s.region_nodes as f64 / s.clicks as f64,
                work / s.units as f64,
                s.freed as f64 / c,
                s.largest_freed,
                s.shaded as f64 / c,
                s.born as f64 / c,
                s.max_slots,
            );
        };
        let mut b = base.clone();
        b.run(UNITS);
        row("atomic (baseline)", &b.stats);
        for pace in with_atomic() {
            let mut r = Run::<true>::new(APP_KEPT, pace).warmed();
            r.run(UNITS);
            let name = match pace {
                Pace::Atomic => "atomic+barriers".into(),
                p => p.name(),
            };
            row(&name, &r.stats);
        }
        println!();
        println!(
            "the costliest unit of the benches' {WINDOW}-unit window by the model (est = 100 a \
             region node, 850 a node built, 75 marked, 40 pruned, 12 a slot tested, 400 freed)"
        );
        println!(
            "  {:<18} {:>5} {:>6} {:>5} {:>6} {:>6} {:>6} {:>5} {:>9}",
            "pace", "unit", "region", "built", "marked", "pruned", "slots", "freed", "est"
        );
        let worst = |name: &str, w: UnitCost| {
            println!(
                "  {:<18} {:>5} {:>6} {:>5} {:>6} {:>6} {:>6} {:>5} {:>9}",
                name,
                w.unit,
                w.region,
                w.allocated,
                w.slice.marked,
                w.slice.pruned,
                w.slice.slots,
                w.slice.freed,
                w.estimate()
            );
        };
        let mut b = base.clone();
        b.run(WINDOW);
        worst("atomic (baseline)", b.stats.worst);
        for pace in PACES {
            let mut r = Run::<true>::new(APP_KEPT, pace).warmed();
            r.run(WINDOW);
            worst(&pace.name(), r.stats.worst);
        }
    }
}
