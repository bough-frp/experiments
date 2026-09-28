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
//!
//! # Follow-ups
//!
//! The first run left a floor: below a budget of about 1,000 a unit the
//! cycle runs long enough that the garbage clicks evaluate meanwhile costs
//! more than slicing saves. Three follow-ups.
//!
//! - **The garbage floor.** Skipping the region's known-dead nodes is
//!   already in place: once the mark ends, a transaction skips white
//!   dependents. The counts measure what it is worth by turning it off
//!   (`keep-dead`, tests only). The floor that remains is floating garbage,
//!   screens abandoned after the mark reached them, which only the next
//!   cycle frees; and the trigger's reference, retaken at the first click
//!   after a cycle, takes that garbage in and never counts it. [`Reset`]
//!   varies when the trigger resets: at the cycle's end, as before; at the
//!   mark's end (`early`), so the work clicks do through the prune and sweep
//!   counts toward the next cycle, which can start the unit after this one
//!   ends; and as `early` with a reference that counts only the region nodes
//!   born before the cycle began (`born`), so floating garbage, born black
//!   during the cycle, counts as excess. That costs a pass over one region
//!   a cycle and a word per node (here, the allocation serial).
//! - **Work debt on the whole region.** [`Pace::RegionDebt`]: each unit of a
//!   cycle adds `c` mark-nodes per region node and per node allocated to a
//!   debt, and the slice pays it. There is no reference, so floating garbage
//!   speeds the collector instead of hiding from it.
//! - **The uneven workload.** [`Uneven`] is `rfd_0003_work_paced_trigger`'s
//!   uneven workload rebuilt on this arena, with the breathing guards on:
//!   four inputs firing at uneven rates, screens kept so live regions grow,
//!   guards dropped through the quiet stretches. Its trigger is that probe's
//!   `Excess` pacer, used as it is; a guard dropped mid-cycle was already
//!   shaded as a root, so the release counts toward the next cycle.

use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use crate::rfd_0003_work_paced_trigger::{Pacer, Policy, UNEVEN_WARMUP, UNEVEN_WINDOW};

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
    /// The same, per barrier, in [`Barrier`]'s order.
    pub shaded_by: [usize; 5],
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

impl Slice {
    /// The work done, in sixteenths of a mark pop, as the budget counts it.
    pub fn work(&self) -> i64 {
        W_MARK * self.marked as i64
            + W_PRUNE * self.pruned as i64
            + W_SLOT * self.slots as i64
            + W_FREE * self.freed as i64
    }
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
    /// Transactions evaluate white dependents after the mark too, as if the
    /// skip weren't there: what the skip saves.
    #[cfg(test)]
    keep_dead: bool,
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

    /// Shades `i` grey if it is white: barrier `b`'s slow path.
    #[inline]
    fn shade(&mut self, b: Barrier, i: Id) {
        let v = &mut self.visit[i as usize];
        if *v != self.epoch {
            *v = self.epoch;
            self.gray.push(i);
            self.cycle.shaded += 1;
            self.cycle.shaded_by[b as usize] += 1;
        }
    }

    /// Whether transactions skip white dependents once the mark has ended.
    #[inline]
    fn skips_dead(&self) -> bool {
        #[cfg(test)]
        if self.keep_dead {
            return false;
        }
        INC && matches!(self.phase, Phase::Prune | Phase::Sweep)
    }

    /// The black nodes, when the mark has just ended: what the cycle will
    /// leave alive, less what is born before it ends.
    pub fn marked_black(&self) -> usize {
        self.reached.len()
    }

    /// The last region's nodes allocated before the node with serial
    /// `serial`: `born`'s reference.
    pub fn region_born_before(&self, serial: u64) -> usize {
        self.order
            .iter()
            .filter(|&&n| self.serial[n as usize] < serial)
            .count()
    }

    /// Gives every root a fresh flag, so that a clone's guards are its own.
    /// Returns the old flags' addresses, mapped to the new ones.
    fn fresh_roots(&mut self) -> HashMap<*const Cell<bool>, Rc<Cell<bool>>> {
        let mut map = HashMap::new();
        for (_, flag) in &mut self.roots {
            let new = Rc::new(Cell::new(flag.get()));
            map.insert(Rc::as_ptr(flag), new.clone());
            *flag = new;
        }
        map
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
                    self.shade(Barrier::Construct, d);
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
            self.shade(Barrier::Hold, t.index);
        }
    }

    /// `depends`: node `i` keeps `targets` alive from now on.
    pub fn depends(&mut self, i: Id, targets: &[Id]) {
        self.reach[i as usize].extend_from_slice(targets);
        if self.barrier(Barrier::Depends) {
            for &d in targets {
                self.shade(Barrier::Depends, d);
            }
        }
    }

    /// Roots node `i` for as long as the returned guard lives (the root
    /// barrier). Dropping the guard touches only its flag.
    pub fn guard(&mut self, i: Id) -> Guard {
        let flag = Rc::new(Cell::new(true));
        self.roots.push((i, flag.clone()));
        if self.barrier(Barrier::Root) {
            self.shade(Barrier::Root, i);
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
            self.shade(Barrier::Switch, new);
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
        if self.skips_dead() {
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
    /// `c` mark-nodes per region node and per node allocated since the
    /// cycle began, less the work done: a debt on the whole region, with no
    /// reference.
    RegionDebt { c: i64 },
}

impl Pace {
    pub fn name(self) -> String {
        match self {
            Pace::Atomic => "atomic".into(),
            Pace::Fixed { k, sliced: false } => format!("k{k}-atomic-sweep"),
            Pace::Fixed { k, sliced: true } => format!("k{k}"),
            Pace::AllocDebt { c } => format!("alloc-debt{c}"),
            Pace::WorkDebt { c } => format!("work-debt{c}"),
            Pace::RegionDebt { c } => format!("region-debt{c}"),
        }
    }

    fn sliced(self) -> bool {
        match self {
            Pace::Atomic => false,
            Pace::Fixed { sliced, .. } => sliced,
            Pace::AllocDebt { .. } | Pace::WorkDebt { .. } | Pace::RegionDebt { .. } => true,
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
    /// Nodes allocated when the trigger was last reset.
    base: u64,
    /// `born`'s reference counts region nodes with a serial below this.
    born_before: u64,
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

    /// Between cycles nothing is freed, so the nodes allocated since the
    /// reset are the live count less the survivors, RFD 3's term.
    #[inline]
    fn due(&self, allocated: u64) -> bool {
        (allocated - self.base) as usize > self.survivors || self.work > self.survivors
    }

    fn collected(&mut self, survivors: usize, allocated: u64) {
        self.survivors = survivors;
        self.work = 0;
        self.reference = None;
        self.base = allocated;
    }
}

/// When the trigger resets and what its reference counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reset {
    /// When the cycle ends; the reference is the next click's region.
    End,
    /// When the mark ends, with the black nodes as the survivors, so the
    /// clicks through the prune and sweep count toward the next cycle.
    Early,
    /// As `Early`, with a reference that counts only the region nodes born
    /// before the cycle began, so floating garbage counts as excess.
    Born,
}

impl Reset {
    /// The suffix a pace's name takes.
    pub fn suffix(self) -> &'static str {
        match self {
            Reset::End => "",
            Reset::Early => "-early",
            Reset::Born => "-born",
        }
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

    /// The collection slice's share of `estimate`.
    pub fn collection(&self) -> usize {
        UnitCost {
            region: 0,
            allocated: 0,
            ..*self
        }
        .estimate()
    }
}

/// What a run has done since its window started.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub units: usize,
    /// Transactions: the clicks in `app`, every input's in `Uneven`.
    pub clicks: usize,
    /// Region nodes over every transaction.
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
    /// `shaded` per barrier, in [`Barrier`]'s order.
    pub shaded_by: [usize; 5],
    /// Audited: unreachable nodes alive when a cycle ends, which only the
    /// next can free, summed over cycles, and the most after one.
    pub floating: usize,
    pub max_floating: usize,
    pub largest_freed: usize,
    pub max_slots: usize,
    /// Guards released.
    pub releases: usize,
    /// The unit the model rates costliest, and the one whose collection
    /// slice it rates costliest.
    pub worst: UnitCost,
    pub worst_slice: UnitCost,
    /// The model's estimate summed over every unit.
    pub estimate: usize,
    /// The sum of the switch's values, which every pace must agree on.
    pub checksum: i64,
}

impl Stats {
    /// Books one unit's cost.
    fn book_unit(&mut self, cost: UnitCost) {
        let e = cost.estimate();
        self.estimate += e;
        if e > self.worst.estimate() {
            self.worst = cost;
        }
        if cost.collection() > self.worst_slice.collection() {
            self.worst_slice = cost;
        }
    }
}

/// Runs a slice of the running cycle and books it. Returns the slice and
/// whether the mark ended in it. With `audit`, checks that no reachable
/// node is white when the mark ends, and counts the floating garbage when
/// the cycle does.
fn book<const INC: bool>(
    arena: &mut Arena<INC>,
    budget: i64,
    sliced: bool,
    audit: bool,
    in_cycle: &mut usize,
    st: &mut Stats,
) -> (Slice, bool) {
    let was_marking = arena.phase() == Phase::Mark;
    let s = arena.slice(budget, sliced);
    let mark_ended = was_marking && arena.phase() != Phase::Mark;
    if audit && mark_ended {
        let lost = arena.white_but_reachable();
        assert!(
            lost.is_empty(),
            "reachable but white at the mark's end: {lost:?}"
        );
    }
    *in_cycle += 1;
    st.collecting_units += 1;
    st.marked += s.marked;
    st.pruned += s.pruned;
    st.slots += s.slots;
    if let Some(c) = s.ended {
        st.cycles += 1;
        st.freed += c.freed;
        st.shaded += c.shaded;
        st.born += c.born;
        for (t, n) in st.shaded_by.iter_mut().zip(c.shaded_by) {
            *t += n;
        }
        st.largest_freed = st.largest_freed.max(c.freed);
        st.longest_cycle = st.longest_cycle.max(*in_cycle);
        if audit {
            let reachable = arena.reachable().iter().filter(|&&r| r).count();
            let floating = arena.live_count() - reachable;
            st.floating += floating;
            st.max_floating = st.max_floating.max(floating);
        }
    }
    (s, mark_ended)
}

/// The `app` workload under one pace: F66's navigation graph with 430
/// screens kept live on the clock beside it.
pub struct Run<const INC: bool> {
    pub arena: Arena<INC>,
    pub pace: Pace,
    pub reset: Reset,
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
    /// The serial the running cycle's first newborn gets.
    cycle_serial: u64,
    /// `RegionDebt`'s debt, in sixteenths of a mark pop.
    debt: i64,
    /// Checks at every mark's end that no reachable node is white, and
    /// counts floating garbage at every cycle's end.
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
        Run::with_reset(kept, pace, Reset::End)
    }

    /// `new`, with the trigger reset as `reset` says.
    pub fn with_reset(kept: usize, pace: Pace, reset: Reset) -> Self {
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
            reset,
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
                base: 0,
                born_before: u64::MAX,
            },
            unit: 0,
            in_cycle: 0,
            cycle_serial: 0,
            debt: 0,
            audit: false,
            stats: Stats::default(),
        };
        r.navigate_to(&mut Rng(0xBA5E));
        r.arena.collect();
        r.arena.reserve_scratch();
        r.arena.transaction(r.clicks, 0);
        r.trigger.survivors = r.arena.live_count();
        r.trigger.base = r.arena.allocated();
        r
    }

    /// The pace's name, with the reset's suffix.
    pub fn name(&self) -> String {
        format!("{}{}", self.pace.name(), self.reset.suffix())
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
        if self.reset == Reset::Born && self.trigger.reference.is_none() {
            let old = self.arena.region_born_before(self.trigger.born_before);
            self.trigger.reference = Some(old);
        }
        let excess = self.trigger.observe(region);
        let s = &mut self.stats;
        s.clicks += 1;
        s.region_nodes += region;
        s.checksum = s.checksum.wrapping_add(self.arena.value(self.switch));
        (region, 0, excess)
    }

    /// After a unit whose click had `region` nodes, which built `allocated`
    /// and grew the region `excess` past its reference: starts a cycle if
    /// none is running and the trigger is due, then runs this unit's slice
    /// of the one running.
    pub fn settle(&mut self, region: usize, allocated: usize, excess: usize) -> Slice {
        self.stats.max_slots = self.stats.max_slots.max(self.arena.slots());
        if self.arena.phase() == Phase::Idle {
            if !self.trigger.due(self.arena.allocated()) {
                return Slice::default();
            }
            self.arena.start_cycle();
            self.in_cycle = 0;
            self.cycle_serial = self.arena.allocated();
        }
        let budget = match self.pace {
            Pace::Atomic => i64::MAX,
            Pace::Fixed { k, .. } => k * W_MARK,
            Pace::AllocDebt { c } => c * allocated as i64 * W_MARK,
            Pace::WorkDebt { c } => c * (allocated + excess) as i64 * W_MARK,
            Pace::RegionDebt { c } => {
                self.debt += c * (region + allocated) as i64 * W_MARK;
                self.debt
            }
        };
        let (s, mark_ended) = book(
            &mut self.arena,
            budget,
            self.pace.sliced(),
            self.audit,
            &mut self.in_cycle,
            &mut self.stats,
        );
        if let Pace::RegionDebt { .. } = self.pace {
            self.debt = if s.ended.is_some() {
                0
            } else {
                self.debt - s.work()
            };
        }
        if mark_ended && self.reset != Reset::End {
            let allocated = self.arena.allocated();
            self.trigger.collected(self.arena.marked_black(), allocated);
            self.trigger.born_before = self.cycle_serial;
        }
        if let Some(c) = s.ended
            && self.reset == Reset::End
        {
            self.trigger.collected(c.survivors, self.arena.allocated());
        }
        s
    }

    /// One whole unit, collection work included.
    pub fn unit(&mut self) -> UnitCost {
        let (region, allocated, excess) = self.step();
        let slice = self.settle(region, allocated, excess);
        let cost = UnitCost {
            unit: self.stats.units - 1,
            region,
            allocated,
            slice,
        };
        self.stats.book_unit(cost);
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
        Run::before_worst_reset(kept, pace, Reset::End, units)
    }

    /// `before_worst`, with the trigger reset as `reset` says.
    pub fn before_worst_reset(kept: usize, pace: Pace, reset: Reset, units: usize) -> Self {
        let start = Run::with_reset(kept, pace, reset).warmed();
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

/// `rfd_0003_work_paced_trigger`'s uneven schedule with the breathing
/// guards on: its first shape (periods of 1,200 units, 900 navigating
/// every third unit, then 300 quiet; a screen kept every 30th unit) with 30
/// screens kept off the tenth input through each navigating stretch and
/// released one guard every 10th unit of the quiet one. Screens hang off
/// the three frequent inputs in turn (its `Mix::Spread`).
mod schedule {
    const PERIOD: usize = 1_200;
    const NAVIGATING: usize = 900;

    fn quiet(k: usize) -> bool {
        k % PERIOD >= NAVIGATING
    }

    pub fn navigates(k: usize) -> bool {
        !quiet(k) && k.is_multiple_of(3)
    }

    pub fn grows(k: usize) -> bool {
        k % 30 == 15
    }

    pub fn inhales(k: usize) -> bool {
        !quiet(k) && k.is_multiple_of(30)
    }

    pub fn exhales(k: usize) -> bool {
        quiet(k) && (k % PERIOD - NAVIGATING).is_multiple_of(10)
    }

    /// Whether input `i` fires in unit `k`: every unit, every tenth, every
    /// hundredth, and once early then every 1,500th.
    pub fn fires(i: usize, k: usize) -> bool {
        match i {
            0 => true,
            1 => k % 10 == 7,
            2 => k % 100 == 53,
            _ => k == 5 || k % 1_500 == 700,
        }
    }

    /// Screens kept live off each input at the start.
    pub const KEPT: [usize; 4] = [20, 20, 20, 370];
}

/// The uneven workload's inputs, by how often they fire.
pub const UNEVEN_INPUTS: [&str; 4] = ["every", "tenth", "hundredth", "rare"];

/// The uneven workload under one pace, atomic or a fixed budget, with the
/// work-paced probe's `Excess` trigger.
pub struct Uneven<const INC: bool> {
    pub arena: Arena<INC>,
    pub pace: Pace,
    inputs: [Id; 4],
    theme: Id,
    nav: Id,
    switch: Id,
    current: Option<Id>,
    navigations: usize,
    grown: usize,
    guards: Vec<Guard>,
    /// Breathing screens' guards, oldest first.
    breathing: VecDeque<Guard>,
    rng: Rng,
    pub pacer: Pacer,
    unit: usize,
    in_cycle: usize,
    /// Guards released while a cycle ran, which it can't free, so they
    /// count toward the next.
    released: usize,
    pub audit: bool,
    pub stats: Stats,
}

impl<const INC: bool> Clone for Uneven<INC> {
    /// A clone whose guards are its own: releasing one in either leaves
    /// the other's roots alone.
    fn clone(&self) -> Self {
        let mut arena = self.arena.clone();
        arena.reserve_scratch();
        let map = arena.fresh_roots();
        let remap = |g: &Guard| Guard(map[&Rc::as_ptr(&g.0)].clone());
        Uneven {
            arena,
            guards: self.guards.iter().map(remap).collect(),
            breathing: self.breathing.iter().map(remap).collect(),
            rng: self.rng.clone(),
            pacer: self.pacer.clone(),
            stats: self.stats.clone(),
            ..*self
        }
    }
}

impl<const INC: bool> Uneven<INC> {
    /// The core, the kept screens, one screen on show, collected: the
    /// work-paced probe's `Uneven::shaped`, seed for seed.
    pub fn new(pace: Pace) -> Self {
        assert!(
            matches!(pace, Pace::Atomic | Pace::Fixed { .. }),
            "atomic or a fixed budget"
        );
        assert!(INC || pace == Pace::Atomic, "no barriers, no increments");
        let mut a = Arena::<INC>::default();
        let mut guards = Vec::new();
        let inputs = [0; 4].map(|_| a.alloc(Op::Input, &[], &[]));
        let navigate = a.alloc(Op::Input, &[], &[]);
        // A theme changes rarely, and every screen reads it.
        let theme = a.alloc(Op::Hold, &[inputs[3]], &[]);
        let nav = a.alloc(Op::Hold, &[navigate], &[]);
        let switch = a.alloc(Op::Switch, &[nav], &[]);
        for i in inputs.into_iter().chain([navigate, switch]) {
            guards.push(a.guard(i));
        }
        let mut rng = Rng(0x5EED_0003);
        for (i, &n) in schedule::KEPT.iter().enumerate() {
            let side = if i == 3 {
                inputs[3]
            } else {
                inputs[(i + 1) % 3]
            };
            for _ in 0..n {
                let last = screen(&mut a, &mut rng, inputs[i], side, theme);
                guards.push(a.guard(last));
            }
        }
        let mut u = Uneven {
            arena: a,
            pace,
            inputs,
            theme,
            nav,
            switch,
            current: None,
            navigations: 0,
            grown: 0,
            guards,
            breathing: VecDeque::new(),
            rng,
            pacer: Pacer::new(Policy::Excess, 0),
            unit: 0,
            in_cycle: 0,
            released: 0,
            audit: false,
            stats: Stats::default(),
        };
        u.navigate();
        u.arena.collect();
        u.pacer = Pacer::new(Policy::Excess, u.arena.live_count());
        u.arena.reserve_scratch();
        u
    }

    /// Runs the work-paced probe's warm-up, two periods, then starts the
    /// window.
    pub fn warmed(mut self) -> Self {
        self.run(UNEVEN_WARMUP);
        self.stats = Stats::default();
        self.arena.reserve_scratch();
        self
    }

    fn navigate(&mut self) {
        let n = self.navigations;
        self.navigations += 1;
        let (src, side) = (self.inputs[n % 3], self.inputs[(n + 1) % 3]);
        let last = screen(&mut self.arena, &mut self.rng, src, side, self.theme);
        self.arena.relink(self.switch, self.current, last);
        let t = self.arena.token(last);
        self.arena.commit(self.nav, Some(t));
        self.current = Some(last);
    }

    /// A screen kept and anchored off a frequent input, in turn.
    fn grow(&mut self) {
        let g = self.grown;
        self.grown += 1;
        let (src, side) = (self.inputs[g % 3], self.inputs[(g + 2) % 3]);
        let last = screen(&mut self.arena, &mut self.rng, src, side, self.theme);
        self.guards.push(self.arena.guard(last));
    }

    fn inhale(&mut self) {
        let (src, side) = (self.inputs[1], self.inputs[3]);
        let last = screen(&mut self.arena, &mut self.rng, src, side, self.theme);
        let g = self.arena.guard(last);
        self.breathing.push_back(g);
    }

    /// The oldest breathing screen's guard dropped, with no runtime access.
    fn exhale(&mut self) {
        if self.breathing.pop_front().is_none() {
            return;
        }
        self.stats.releases += 1;
        if self.arena.phase() == Phase::Idle {
            self.pacer.release();
        } else {
            self.released += 1;
        }
    }

    /// One unit's work, without collection. Returns whether it navigated,
    /// and its transactions' region nodes.
    pub fn step(&mut self) -> (bool, usize) {
        let k = self.unit;
        self.unit += 1;
        self.stats.units += 1;
        let navigated = schedule::navigates(k);
        if navigated {
            self.navigate();
        }
        if schedule::grows(k) {
            self.grow();
        }
        if schedule::inhales(k) {
            self.inhale();
        }
        if schedule::exhales(k) {
            self.exhale();
        }
        let mut region = 0;
        for i in 0..4 {
            if schedule::fires(i, k) {
                let input = self.inputs[i];
                self.arena.transaction(input, k as i64);
                let r = self.arena.region().len();
                self.pacer.observe(input, r);
                region += r;
                let s = &mut self.stats;
                s.clicks += 1;
                s.checksum = s.checksum.wrapping_add(self.arena.value(self.switch));
            }
        }
        self.stats.region_nodes += region;
        (navigated, region)
    }

    /// After a unit: starts a cycle if none is running and the trigger is
    /// due, then runs this unit's slice of the one running.
    pub fn settle(&mut self, navigated: bool) -> Slice {
        self.stats.max_slots = self.stats.max_slots.max(self.arena.slots());
        if self.arena.phase() == Phase::Idle {
            if !self.pacer.due(self.arena.live_count(), navigated) {
                return Slice::default();
            }
            self.arena.start_cycle();
            self.in_cycle = 0;
        }
        let budget = match self.pace {
            Pace::Fixed { k, .. } => k * W_MARK,
            _ => i64::MAX,
        };
        let (s, _) = book(
            &mut self.arena,
            budget,
            self.pace.sliced(),
            self.audit,
            &mut self.in_cycle,
            &mut self.stats,
        );
        if let Some(c) = s.ended {
            self.pacer.collected(c.survivors, c.freed);
            for _ in 0..std::mem::take(&mut self.released) {
                self.pacer.release();
            }
        }
        s
    }

    /// One whole unit, collection work included.
    pub fn unit(&mut self) -> UnitCost {
        let before = self.arena.allocated();
        let (navigated, region) = self.step();
        let allocated = (self.arena.allocated() - before) as usize;
        let slice = self.settle(navigated);
        let cost = UnitCost {
            unit: self.stats.units - 1,
            region,
            allocated,
            slice,
        };
        self.stats.book_unit(cost);
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
    /// costliest, stopped right before it.
    pub fn before_worst(pace: Pace, units: usize) -> Self {
        let start = Uneven::new(pace).warmed();
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
}

/// The uneven workload's measured window.
pub const UNEVEN_UNITS: usize = UNEVEN_WINDOW;

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

    /// A row of the extended tables: the run's stats over its whole run,
    /// and the worst units of its first `WINDOW` (or `UNEVEN_WINDOW`).
    fn ext_row(name: &str, s: &Stats, worst: (UnitCost, UnitCost), atomic: f64) {
        let work = s.marked as f64
            + s.pruned as f64 / 2.0
            + s.slots as f64 * 3.0 / 16.0
            + s.freed as f64 * 5.0;
        let c = s.cycles.max(1) as f64;
        let u = s.units as f64;
        let est = s.estimate as f64 / u;
        println!(
            "  {:<18} {:>6} {:>6.1} {:>6} {:>8.1} {:>7.1} {:>7.0} {:>6.3} {:>8} {:>8} {:>7.1} {:>6} {:>6.1} {:>6.1}",
            name,
            s.cycles,
            s.collecting_units as f64 / c,
            s.longest_cycle,
            s.region_nodes as f64 / s.clicks as f64,
            work / u,
            est,
            est / atomic,
            worst.0.estimate(),
            worst.1.collection(),
            s.floating as f64 / c,
            s.max_floating,
            s.shaded as f64 / c,
            s.born as f64 / c,
        );
    }

    fn ext_header(region: &str) {
        println!(
            "  {:<18} {:>6} {:>6} {:>6} {:>8} {:>7} {:>7} {:>6} {:>8} {:>8} {:>7} {:>6} {:>6} {:>6}",
            "pace",
            "cycles",
            "u/cyc",
            "maxcyc",
            region,
            "coll/u",
            "est/u",
            "/atom",
            "worst",
            "wcoll",
            "float",
            "maxfl",
            "shaded",
            "born"
        );
    }

    fn ext_legend() {
        println!(
            "columns: u/cyc, maxcyc = units a cycle runs, mean and longest; coll/u = collection \
             work a unit in mark-node equivalents; est/u = the model's estimate of a unit's \
             instructions (100 a region node, 850 a node built, 75 marked, 40 pruned, 12 a slot \
             tested, 400 freed), and /atom that over the atomic baseline's; worst = the costliest \
             unit of the benches' window by the model, wcoll = the costliest collection slice \
             alone; float = unreachable nodes alive when a cycle ends (floating garbage the next \
             frees), per cycle, and maxfl the most; shaded = white nodes barriers shaded, born = \
             nodes born black, per cycle"
        );
    }

    /// The follow-ups on `app`: the garbage floor (what skipping known-dead
    /// region nodes saves, and the trigger reset early or counting floating
    /// garbage), and work debt on the whole region, against fixed budgets.
    #[test]
    fn extended_counts_app() {
        const UNITS: usize = 30_000;
        let base = Run::<false>::new(APP_KEPT, Pace::Atomic).warmed();
        println!(
            "app: {} live after a collection; {UNITS} units from each pace's first cycle, every \
             third a navigation, the rest a click; worst and wcoll over the first {WINDOW}",
            base.arena.live_count()
        );
        println!(
            "trigger: the work-paced probe's `excess`, reset when the cycle ends; -early: reset \
             when the mark ends (survivors = black nodes), so the next cycle can start the unit \
             after this one ends; -born: -early with a reference of the click's region nodes \
             born before the cycle began; keep-dead: transactions evaluate white dependents \
             after the mark too; region-debt<c>: c mark-nodes per region node and per node built \
             since the cycle began, less the work done"
        );
        ext_legend();
        ext_header("rgn/clk");
        let measure = |mut r: Run<true>| {
            r.audit = true;
            r.run(WINDOW);
            let worst = (r.stats.worst, r.stats.worst_slice);
            r.run(UNITS - WINDOW);
            (r.name(), r.stats, worst)
        };
        let mut b = base.clone();
        b.audit = true;
        b.run(WINDOW);
        let worst = (b.stats.worst, b.stats.worst_slice);
        b.run(UNITS - WINDOW);
        let atomic = b.stats.estimate as f64 / b.stats.units as f64;
        ext_row("atomic (baseline)", &b.stats, worst, atomic);
        let fixed = |k| Pace::Fixed { k, sliced: true };
        let mut rows = Vec::new();
        for k in [250, 500, 1_000, 2_000, 4_000] {
            rows.push((fixed(k), Reset::End, false));
        }
        for k in [250, 1_000] {
            rows.push((fixed(k), Reset::End, true));
        }
        for k in [250, 1_000, 4_000] {
            rows.push((fixed(k), Reset::Early, false));
        }
        for k in [250, 500, 1_000, 4_000] {
            rows.push((fixed(k), Reset::Born, false));
        }
        for c in [1, 2, 4, 8] {
            rows.push((Pace::RegionDebt { c }, Reset::End, false));
        }
        for c in [2, 4] {
            rows.push((Pace::RegionDebt { c }, Reset::Born, false));
        }
        for (pace, reset, keep_dead) in rows {
            let mut r = Run::<true>::with_reset(APP_KEPT, pace, reset);
            r.arena.keep_dead = keep_dead;
            let (name, stats, worst) = measure(r.warmed());
            let name = if keep_dead {
                format!("{name}-keep-dead")
            } else {
                name
            };
            ext_row(&name, &stats, worst, atomic);
        }
    }

    /// The uneven workload with guards dropped under the incremental
    /// collector: barriers' slow paths, floating garbage, worst pause and
    /// total cost against atomic; then whether each pace leaves exactly
    /// the atomic collector's live set once the running cycle and one more
    /// have run.
    #[test]
    fn extended_counts_uneven() {
        let base = Uneven::<false>::new(Pace::Atomic);
        println!(
            "uneven (the work-paced probe's, breathing): {} live after the first collection; \
             {UNEVEN_WINDOW} units after {UNEVEN_WARMUP} of warm-up; periods of 1,200 units, 900 \
             navigating every third unit then 300 quiet; a screen kept every 30th unit; 30 kept \
             off the tenth input through each navigating stretch, a guard released every 10th \
             quiet unit",
            base.arena.live_count()
        );
        println!(
            "inputs fire every unit, every 10th, every 100th, and at unit 5 then every 1,500th; \
             screens hang off the three frequent ones in turn; trigger: the work-paced probe's \
             `Excess` pacer, a release mid-cycle counted toward the next"
        );
        ext_legend();
        println!("  rgn/u = region nodes a unit, over every input");
        ext_header("rgn/u");
        let paces = [
            Pace::Atomic,
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
        ];
        let mut b = base.clone().warmed();
        b.audit = true;
        b.run(UNEVEN_WINDOW);
        let s = &b.stats;
        let atomic = s.estimate as f64 / s.units as f64;
        let mut runs: Vec<(String, Stats)> = vec![("atomic (baseline)".into(), b.stats.clone())];
        for pace in paces {
            let mut r = Uneven::<true>::new(pace).warmed();
            r.audit = true;
            r.run(UNEVEN_WINDOW);
            let name = match pace {
                Pace::Atomic => "atomic+barriers".into(),
                p => p.name(),
            };
            runs.push((name, r.stats));
        }
        for (name, s) in &runs {
            // Region nodes a unit, not a transaction: `clicks` is units.
            let mut per_unit = s.clone();
            per_unit.clicks = per_unit.units;
            ext_row(name, &per_unit, (s.worst, s.worst_slice), atomic);
        }
        println!();
        println!(
            "per cycle: white nodes each barrier shaded (construct, switch, hold, depends, \
             root); guards released in the window"
        );
        for (name, s) in &runs {
            let c = s.cycles.max(1) as f64;
            let by = s.shaded_by.map(|n| format!("{:.2}", n as f64 / c));
            println!(
                "  {:<18} {:>6} {:>6} {:>6} {:>6} {:>6}   {} total, {} released",
                name, by[0], by[1], by[2], by[3], by[4], s.shaded, s.releases
            );
        }
        println!();
        println!("the costliest unit of the window by the model, and the costliest slice");
        println!(
            "  {:<18} {:>5} {:>6} {:>5} {:>6} {:>6} {:>6} {:>5} {:>9}",
            "pace", "unit", "region", "built", "marked", "pruned", "slots", "freed", "est"
        );
        for (name, s) in &runs {
            for (tag, w) in [("", s.worst), (" slice", s.worst_slice)] {
                println!(
                    "  {:<18} {:>5} {:>6} {:>5} {:>6} {:>6} {:>6} {:>5} {:>9}",
                    format!("{name}{tag}"),
                    w.unit,
                    w.region,
                    w.allocated,
                    w.slice.marked,
                    w.slice.pruned,
                    w.slice.slots,
                    w.slice.freed,
                    w.estimate()
                );
            }
        }
        println!();
        const UNITS: usize = UNEVEN_WARMUP + UNEVEN_WINDOW;
        let mut atomic = base;
        atomic.audit = true;
        let sum = atomic.run(UNITS);
        atomic.drain();
        let alive = atomic.arena.live_serials();
        assert_eq!(alive, atomic.arena.reachable_serials());
        println!(
            "exactness over {UNITS} units, audited at every mark's end; then the running cycle \
             and one more: atomic leaves {} alive, exactly the reachable",
            alive.len()
        );
        for pace in paces {
            let mut r = Uneven::<true>::new(pace);
            r.audit = true;
            assert_eq!(r.run(UNITS), sum, "{pace:?}: values");
            let cycles = r.stats.cycles;
            r.drain();
            let same = r.arena.live_serials() == alive;
            println!(
                "  {:<18} {} cycles, values as atomic; live set after draining {}",
                pace.name(),
                cycles,
                if same { "equals atomic's" } else { "DIFFERS" }
            );
            assert!(same, "{pace:?}: live set");
        }
    }
}
