//! Does restricting a transaction's mark to nodes flagged live from the
//! roots cost less per transaction than evaluating garbage until it is
//! collected (F66), once the flag's own upkeep is paid?
//!
//! RFD 5 marks the affected region depth first over dependents lists and
//! evaluates it in a flat loop. RFD 3 accepts that a node no root reaches is
//! still in its inputs' dependents lists, so it is marked and evaluated
//! whenever they fire, until a collection frees it and prunes it out. F66
//! measured that on the spike: after 9,000 abandoned screens a navigation
//! cost 596 µs a transaction without collection and 528 ns with it. The
//! literature's answer is to bound evaluation by liveness from the roots
//! (synthesis 02, "Candidate probes").
//!
//! # Which variants mean anything
//!
//! *A flag refreshed only at collection buys nothing.* A collection frees
//! every node it finds dead, so right after one no dead node is left to
//! skip, and every node the flag would skip later has died since, which the
//! flag can't know. The proposal as written is `garbage` plus a test. What
//! the flag can buy is a liveness pass cheaper than a collection, run when a
//! collection isn't due. The mark is the part of a collection that decides
//! liveness; the sweep (every slot tested, the dead freed and bumped) only
//! reclaims. So:
//!
//! - **mark only, `Refresh::Mark`**: the collection's mark alone, which
//!   stamps every reached node with the epoch; a node allocated later is
//!   stamped at birth, so it counts as live. The transaction tests each
//!   dependent's stamp and skips the unstamped (`Push::Skip`). The
//!   dependents lists are untouched, so a dead entry costs a test every
//!   time its input fires, until a collection prunes it.
//! - **census, `Refresh::Census`**: the mark, then the dead taken out of
//!   the reached nodes' dependents lists. That is a collection without the
//!   sweep. Afterwards the transaction is RFD 5's, with no test at all,
//!   since nothing a fired input reaches over dependents is dead. The dead
//!   stay allocated until a collection frees them; the census defers that
//!   cost, it doesn't remove it.
//!
//! *A flag maintained incrementally when a guard drops or a switch moves* is
//! not built. Deciding that a dropped edge's target is dead means deciding
//! that nothing else reaches it, which without counts (RFD 3 has none, for
//! cycles' sake) is a walk back over every edge into it looking for a root:
//! dependencies, the cells that snapshot it and every value holding its
//! token, the last two of which the arena doesn't index backwards. In the
//! worst case that walk is the live graph, a census. So the incremental
//! flag is a census with extra indexes; this probe measures the census and
//! leaves the incremental version as that bound.
//!
//! # Safety, which RFD 3 fixes
//!
//! A census is a collection whose sweep is deferred, so a node it finds
//! dead must be dead everywhere at once, not just skipped: its token must
//! fail its check, and the next mark must not follow a held token to it.
//! Otherwise a cell that came to hold a stale token (an undeclared capture,
//! F62) would bring the node back, flagged live but pruned out of its
//! inputs' lists, and it would silently stop computing. So [`Arena::lookup`]
//! and the mark's token check both test the stamp. A node reachable at the
//! census is stamped, and a node allocated later is stamped at birth, so a
//! reachable node is never skipped; a node the census didn't reach can't be
//! observed again, which is what makes skipping it safe.
//!
//! # The model
//!
//! The arena is `rfd_0003_sweep_cost`'s, trimmed and extended: parallel
//! vectors indexed by `u32`, boxed payloads so a free is a deallocation,
//! guards as roots, the collection's visit stamp doubling as the flag. The
//! collection prunes the reached nodes' lists rather than walking every
//! slot, so a collection is exactly a census plus a sweep.
//!
//! The workload is F66's shape with RFD 4's screens. A core the I/O code
//! anchors: the clicks input, a clock input, a theme (a hold over the
//! clock) and the navigation input. A `nav` hold holds the token of the
//! current screen, and a switch over it, which the one listener roots, is
//! linked under that screen's last node. Each navigation builds a screen of
//! 18 to 30 nodes hanging off the clicks, the clock and the theme, moves
//! the switch onto it, and so leaves the old screen reachable from nothing.
//! A transaction fires the clicks. An `app` fixture adds screens hanging
//! off the clock and the theme, kept live by anchors, for a live set of
//! about ten thousand nodes: the case where a liveness pass is not cheap,
//! and the only one where RFD 3's trigger lets garbage build up (see the
//! wall-clock bench's doc).

use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;

/// A node's index in the arena.
pub type Id = u32;

/// The graph id every token of this model carries.
const GRAPH: u32 = 1;

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

/// How a transaction's mark treats a dependent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Push {
    /// RFD 5 as written: every dependent is marked.
    All,
    /// A dependent the last mark didn't stamp is skipped.
    Skip,
}

/// What runs between units to keep the dead out of transactions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refresh {
    /// Nothing: F66's case.
    None,
    /// The collection's mark alone, which refreshes the flag.
    Mark,
    /// The mark, then the dead out of the reached nodes' dependents lists.
    Census,
    /// A whole collection: mark, prune, sweep.
    Collect,
}

impl Refresh {
    /// The push a transaction needs after this refresh.
    pub fn push(self) -> Push {
        match self {
            Refresh::Mark => Push::Skip,
            _ => Push::All,
        }
    }
}

/// A listener or an anchor as I/O code holds it: a root while it lives.
pub struct Guard(Rc<Cell<bool>>);

impl Drop for Guard {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

/// The arena, its roots, and the scratch refreshes and transactions reuse.
#[derive(Clone, Default)]
pub struct Arena {
    generation: Vec<u32>,
    live: Vec<bool>,
    /// The epoch of the last mark that reached the node, or of its birth:
    /// the flag. A node is flagged live when this equals `epoch`.
    visit: Vec<u32>,
    /// The transaction's mark stamp.
    mark: Vec<u32>,
    deps: Vec<Vec<Id>>,
    /// Marking a transaction follows these.
    dependents: Vec<Vec<Id>>,
    /// Reach that is not a dependency: the cell a snapshot reads.
    reach: Vec<Vec<Id>>,
    /// The token a cell's committed value holds, which the mark traces.
    held: Vec<Option<Token>>,
    op: Vec<Option<Box<Op>>>,
    value: Vec<i64>,
    /// Freed slots, oldest first.
    free: VecDeque<Id>,
    live_count: usize,
    allocated: usize,
    roots: Vec<(Id, Rc<Cell<bool>>)>,
    epoch: u32,
    tx: u32,
    gray: Vec<Id>,
    /// The last mark's reached nodes.
    reached: Vec<Id>,
    order: Vec<Id>,
    stack: Vec<(Id, usize)>,
}

impl Arena {
    /// Slots in the arena, live or free.
    pub fn slots(&self) -> usize {
        self.live.len()
    }

    /// Nodes allocated and not yet freed, dead or alive.
    pub fn live_count(&self) -> usize {
        self.live_count
    }

    /// Nodes the flag says are live.
    pub fn flagged(&self) -> usize {
        (0..self.slots())
            .filter(|&n| self.live[n] && self.visit[n] == self.epoch)
            .count()
    }

    /// The last transaction's region, in evaluation order reversed.
    pub fn region(&self) -> &[Id] {
        &self.order
    }

    pub fn value(&self, i: Id) -> i64 {
        self.value[i as usize]
    }

    fn is_flagged(&self, i: Id) -> bool {
        self.visit[i as usize] == self.epoch
    }

    /// A slot for a new node, the oldest freed one or a new one, stamped
    /// with the current epoch: a node born after a census is live to it.
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
                i
            }
        };
        let n = i as usize;
        self.visit[n] = self.epoch;
        self.live_count += 1;
        self.allocated += 1;
        self.deps[n].extend_from_slice(deps);
        self.reach[n].extend_from_slice(reach);
        for &d in deps {
            self.dependents[d as usize].push(i);
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

    /// Validates a token: graph id, liveness, generation, and the flag, so
    /// a node the last census found dead is as gone as a freed one.
    pub fn lookup(&self, t: Token) -> Option<Id> {
        let n = t.index as usize;
        let ok = t.graph == GRAPH
            && n < self.live.len()
            && self.live[n]
            && self.generation[n] == t.generation
            && self.is_flagged(t.index);
        ok.then_some(t.index)
    }

    /// Sets the token cell `i`'s committed value holds.
    pub fn hold_token(&mut self, i: Id, t: Token) {
        self.held[i as usize] = Some(t);
    }

    /// Roots node `i` for as long as the returned guard lives.
    pub fn guard(&mut self, i: Id) -> Guard {
        let flag = Rc::new(Cell::new(true));
        self.roots.push((i, flag.clone()));
        Guard(flag)
    }

    /// Moves switch `sw`, whose first dependency is its outer, from inner
    /// `old` (if any) to inner `new`: RFD 5's relink.
    pub fn relink(&mut self, sw: Id, old: Option<Id>, new: Id) {
        if let Some(old) = old {
            self.dependents[old as usize].retain(|&d| d != sw);
        }
        let deps = &mut self.deps[sw as usize];
        deps.truncate(1);
        deps.push(new);
        self.dependents[new as usize].push(sw);
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

    /// Runs a refresh. Returns how many nodes it reached, or for a
    /// collection how many it freed.
    pub fn refresh(&mut self, r: Refresh) -> usize {
        match r {
            Refresh::None => 0,
            Refresh::Mark => self.mark_roots(),
            Refresh::Census => self.census(),
            Refresh::Collect => self.collect(),
        }
    }

    /// The mark, then the dead out of every reached node's dependents list.
    /// A collection without its sweep.
    pub fn census(&mut self) -> usize {
        let reached = self.mark_roots();
        self.prune();
        reached
    }

    /// One collection: mark, prune, sweep. Returns how many it freed.
    pub fn collect(&mut self) -> usize {
        self.mark_roots();
        self.prune();
        self.sweep()
    }

    /// Stamps everything the live guards reach, through dependencies,
    /// recorded reach and held tokens, with a fresh epoch, and records what
    /// it reached. Returns how many nodes it reached.
    pub fn mark_roots(&mut self) -> usize {
        let was = self.epoch;
        self.epoch += 1;
        let epoch = self.epoch;
        self.roots.retain(|(_, flag)| flag.get());
        let Arena {
            generation,
            live,
            visit,
            deps,
            reach,
            held,
            roots,
            gray,
            reached,
            ..
        } = self;
        reached.clear();
        let shade = |visit: &mut Vec<u32>, gray: &mut Vec<Id>, i: Id| {
            let v = &mut visit[i as usize];
            if *v != epoch {
                *v = epoch;
                gray.push(i);
            }
        };
        for &(i, _) in roots.iter() {
            shade(visit, gray, i);
        }
        while let Some(n) = gray.pop() {
            reached.push(n);
            let at = n as usize;
            for &d in &deps[at] {
                shade(visit, gray, d);
            }
            for &r in &reach[at] {
                shade(visit, gray, r);
            }
            // A token that fails its check names nothing, and a node the
            // last mark didn't stamp is as dead as a freed one: following
            // the token would bring it back unlisted in its inputs.
            if let Some(t) = held[at] {
                let m = t.index as usize;
                if t.graph == GRAPH
                    && m < live.len()
                    && live[m]
                    && generation[m] == t.generation
                    && (visit[m] == was || visit[m] == epoch)
                {
                    shade(visit, gray, t.index);
                }
            }
        }
        reached.len()
    }

    /// Takes every node the last mark didn't stamp out of the reached
    /// nodes' dependents lists, keeping the order of the rest. The dead's
    /// own lists are never walked again.
    fn prune(&mut self) {
        let Arena {
            visit,
            dependents,
            reached,
            epoch,
            ..
        } = self;
        for &n in reached.iter() {
            dependents[n as usize].retain(|&d| visit[d as usize] == *epoch);
        }
    }

    /// Frees every allocated node the last mark didn't stamp, in index
    /// order: drops its payload and held token, bumps its generation, and
    /// puts it on the back of the free list. Returns how many it freed.
    fn sweep(&mut self) -> usize {
        let epoch = self.epoch;
        let mut freed = 0;
        for n in 0..self.live.len() {
            if self.live[n] && self.visit[n] != epoch {
                self.live[n] = false;
                self.op[n] = None;
                self.held[n] = None;
                self.generation[n] += 1;
                self.free.push_back(n as Id);
                freed += 1;
            }
        }
        self.live_count -= freed;
        freed
    }

    /// One transaction: sends `x` to `input`, marks what depends on it
    /// depth first over dependents (skipping the unflagged under
    /// `Push::Skip`), and evaluates the reverse post-order. Returns the
    /// last value evaluated.
    pub fn transaction(&mut self, input: Id, x: i64, push: Push) -> i64 {
        match push {
            Push::All => self.mark_region::<false>(input),
            Push::Skip => self.mark_region::<true>(input),
        }
        self.evaluate(x)
    }

    fn mark_region<const SKIP: bool>(&mut self, input: Id) {
        assert!(self.is_flagged(input), "a send to a dead input is stale");
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

    fn evaluate(&mut self, x: i64) -> i64 {
        let mut last = 0;
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
            last = v;
        }
        last
    }
}

/// A small fixed-seed generator, SplitMix64, so every run builds the same
/// graph.
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

    /// Uniform in `lo..=hi`, near enough.
    fn range(&mut self, lo: u32, hi: u32) -> u32 {
        lo + (self.next() % u64::from(hi - lo + 1)) as u32
    }
}

/// A screen's last node, which the switch selects, and its size.
#[derive(Clone, Copy, Debug)]
pub struct Screen {
    pub last: Id,
    pub nodes: usize,
}

/// Builds one screen: branches of maps off `src` (the first also reading
/// the theme, the second `side`), merged, held, snapshotted, and a cell
/// holding the token of a node that depends on it, a cycle through values.
fn screen(a: &mut Arena, rng: &mut Rng, src: Id, side: Id, theme: Id) -> Screen {
    let before = a.allocated;
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
    a.hold_token(select, t);
    let last = a.alloc(Op::Map(0), &[count, select], &[]);
    Screen {
        last,
        nodes: a.allocated - before,
    }
}

/// F66's navigation graph after some navigations, with whatever refresh
/// ran after each. Cloning it gives every measured run the same start.
pub struct Fixture {
    pub arena: Arena,
    /// The input a transaction fires.
    pub clicks: Id,
    /// The switch the listener roots, whose value a transaction yields.
    pub switch: Id,
    clock: Id,
    theme: Id,
    nav: Id,
    current: Option<Screen>,
    /// Screens built by navigation and left since.
    pub abandoned: usize,
    /// Nodes of the screens kept live besides the current one.
    pub kept_nodes: usize,
    rng: Rng,
    guards: Rc<Vec<Guard>>,
}

impl Clone for Fixture {
    fn clone(&self) -> Fixture {
        let mut arena = self.arena.clone();
        arena.reserve_scratch();
        Fixture {
            arena,
            rng: self.rng.clone(),
            guards: self.guards.clone(),
            ..*self
        }
    }
}

impl Fixture {
    /// The core, `kept` screens off the clock kept live by anchors, then
    /// `abandoned` navigations to screens of random shape, each followed by
    /// `refresh`, and a last navigation, followed by `refresh`, to the
    /// screen every fixture shows, the same shape at every size.
    pub fn new(kept: usize, abandoned: usize, refresh: Refresh) -> Fixture {
        let mut a = Arena::default();
        let mut guards = Vec::new();
        // The core the I/O code anchors.
        let clicks = a.alloc(Op::Input, &[], &[]);
        let clock = a.alloc(Op::Input, &[], &[]);
        let navigate = a.alloc(Op::Input, &[], &[]);
        let theme = a.alloc(Op::Hold, &[clock], &[]);
        let nav = a.alloc(Op::Hold, &[navigate], &[]);
        let switch = a.alloc(Op::Switch, &[nav], &[]);
        for i in [clicks, clock, navigate] {
            guards.push(a.guard(i));
        }
        guards.push(a.guard(switch));
        let mut rng = Rng(0x5EED_0005);
        let mut kept_nodes = 0;
        for _ in 0..kept {
            let s = screen(&mut a, &mut rng, clock, theme, theme);
            kept_nodes += s.nodes;
            guards.push(a.guard(s.last));
        }
        let mut f = Fixture {
            arena: a,
            clicks,
            switch,
            clock,
            theme,
            nav,
            current: None,
            abandoned: 0,
            kept_nodes,
            rng,
            guards: Rc::new(guards),
        };
        for _ in 0..abandoned {
            f.navigate();
            f.arena.refresh(refresh);
        }
        f.navigate_to(&mut Rng(0xBA5E));
        f.arena.refresh(refresh);
        f.arena.reserve_scratch();
        f
    }

    /// Runs `r` once.
    pub fn refreshed(mut self, r: Refresh) -> Fixture {
        self.arena.refresh(r);
        self
    }

    /// Navigates once more, to a screen of random shape, with no refresh.
    pub fn navigated(mut self) -> Fixture {
        self.navigate();
        self
    }

    /// Runs one transaction, so the scratch and the values are warm.
    pub fn warmed(mut self, push: Push) -> Fixture {
        self.click(0, push);
        self
    }

    /// A navigation: a construct builds a screen, `nav` holds its token and
    /// the switch moves onto it, which leaves the old screen unreachable.
    pub fn navigate(&mut self) {
        let mut rng = self.rng.clone();
        self.navigate_to(&mut rng);
        self.rng = rng;
    }

    fn navigate_to(&mut self, rng: &mut Rng) {
        let s = screen(&mut self.arena, rng, self.clicks, self.clock, self.theme);
        let old = self.current.map(|c| c.last);
        self.arena.relink(self.switch, old, s.last);
        let t = self.arena.token(s.last);
        self.arena.hold_token(self.nav, t);
        if self.current.is_some() {
            self.abandoned += 1;
        }
        self.current = Some(s);
    }

    /// The nodes of the screen on show.
    pub fn current_nodes(&self) -> usize {
        self.current.map_or(0, |s| s.nodes)
    }

    /// One transaction on the clicks. Returns the switch's value.
    pub fn click(&mut self, x: i64, push: Push) -> i64 {
        self.arena.transaction(self.clicks, x, push);
        self.arena.value(self.switch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REFRESHES: [Refresh; 4] = [
        Refresh::None,
        Refresh::Mark,
        Refresh::Census,
        Refresh::Collect,
    ];

    /// Every refresh gives the switch the same values through navigations
    /// and clicks, and every refresh but none keeps the region to the
    /// screen on show, the switch and the clicks.
    #[test]
    fn every_refresh_computes_the_same_values() {
        for kept in [0, 20] {
            let mut runs = Vec::new();
            for r in REFRESHES {
                let mut f = Fixture::new(kept, 30, r);
                let mut seen = Vec::new();
                let mut regions = Vec::new();
                for k in 0..40 {
                    if k % 3 == 2 {
                        f.navigate();
                        f.arena.refresh(r);
                    }
                    seen.push(f.click(k, r.push()));
                    regions.push(f.arena.region().len());
                }
                if r != Refresh::None {
                    for (k, &n) in regions.iter().enumerate() {
                        assert!(n <= 32, "{r:?} click {k}: region of {n}");
                    }
                }
                eprintln!(
                    "kept {kept} {r:?}: regions {:?}..{:?}",
                    &regions[..3],
                    &regions[37..]
                );
                runs.push(seen);
            }
            for run in &runs[1..] {
                assert_eq!(run, &runs[0]);
            }
        }
    }

    /// A census flags exactly what a collection keeps, its dead fail their
    /// token checks, a collection after it frees exactly them, and a live
    /// cell holding a dead node's token doesn't bring it back.
    #[test]
    fn a_census_is_a_collection_without_its_sweep() {
        for (kept, abandoned) in [(0, 100), (400, 100), (0, 1_000)] {
            let f = Fixture::new(kept, abandoned, Refresh::None);
            // A node of an abandoned screen: past the core and the kept
            // screens, and well before the last navigation's.
            let old = f.arena.token(f.kept_nodes as Id + 100);
            assert!(f.arena.lookup(old).is_some());
            let mut collected = f.clone();
            let freed = collected.arena.collect();
            let survivors = collected.arena.live_count();
            let mut census = f.clone();
            census.arena.census();
            assert_eq!(census.arena.flagged(), survivors);
            assert_eq!(census.arena.lookup(old), None);
            census.arena.hold_token(census.theme, old);
            census.arena.mark_roots();
            assert_eq!(census.arena.flagged(), survivors);
            assert_eq!(census.arena.collect(), freed);
            let mut pruned = f.clone().refreshed(Refresh::Census);
            pruned.click(1, Push::All);
            assert!(
                pruned
                    .arena
                    .region()
                    .iter()
                    .all(|&n| pruned.arena.is_flagged(n))
            );
            let mut garbage = f.clone();
            garbage.click(1, Push::All);
            eprintln!(
                "kept {kept} ({} nodes), {abandoned} abandoned: {} slots, {survivors} live, \
                 {freed} garbage ({:.1} a screen), current screen {} nodes, region {} with \
                 garbage, {} without",
                f.kept_nodes,
                f.arena.slots(),
                freed as f64 / abandoned as f64,
                f.current_nodes(),
                garbage.arena.region().len(),
                pruned.arena.region().len(),
            );
        }
    }
}
