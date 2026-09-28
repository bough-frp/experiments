//! How long does one mark-sweep of Bough's arena take, at a thousand, ten
//! thousand and a hundred thousand slots with a tenth and nine tenths of
//! them live, and how does that compare with the unit it follows?
//!
//! RFD 3 collects between units: mark from the roots over reach, sweep the
//! arena, bump the generation of every freed slot and put it on the back
//! of a first-in first-out free list (or retire it at the maximum
//! generation), and prune the dead out of the survivors' dependents lists.
//! The mark costs in proportion to what is live; the sweep and the prune
//! walk every slot. How often the trigger can afford to collect (after
//! every unit once a threshold is passed, or paced by debt so the cost is
//! spread) depends on those two costs against a unit's (F66).
//!
//! This module is a model of the spike's arena in the few hundred lines the
//! question needs: parallel vectors indexed by `u32` (generation, liveness,
//! the collection's visit stamp, the transaction's mark stamp,
//! dependencies, dependents, reach, the token a cell's value holds, and a
//! boxed node payload whose free is a deallocation, as the spike's drop of
//! a node's data and program is), roots as a list of guards, and the
//! transaction's mark-and-evaluate over dependents that the benches use as
//! the baseline. Values are integers; nothing else here needs one.
//!
//! The graph is UI-like: a small core the application anchors (a clock
//! input and two cells every screen reads, a theme and a locale), and
//! screens, each an input fanning out to chains of maps, merged, held,
//! snapshotted, and a cell whose value is a token naming a node that
//! depends on it, a cycle through values. One listener on each screen's
//! last node roots the screen. The live fraction is set by which screens'
//! listeners are kept; the rest are dropped before the collection.

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
/// deallocation, as it is in the spike, where a free drops the node's data
/// and its program.
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
}

/// The runtime's side of a guard: the flag the user's side clears on drop,
/// so neither needs the other.
#[derive(Clone)]
struct Liveness(Rc<Cell<bool>>);

/// A listener or an anchor as I/O code holds it: a root while it lives.
pub struct Guard(Rc<Cell<bool>>);

impl Drop for Guard {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

/// The arena, its roots, and the scratch collection and transactions
/// reuse. Parallel vectors indexed by `Id`. A clone shares the guards'
/// flags, so it sees the same roots.
#[derive(Clone, Default)]
pub struct Arena {
    generation: Vec<u32>,
    live: Vec<bool>,
    /// The collection's visit stamp.
    visit: Vec<u32>,
    /// The transaction's mark stamp.
    mark: Vec<u32>,
    deps: Vec<Vec<Id>>,
    /// Marking a transaction follows these.
    dependents: Vec<Vec<Id>>,
    /// Reach that is not a dependency: the cell a snapshot reads.
    reach: Vec<Vec<Id>>,
    /// The token a cell's committed value holds, which collection traces.
    held: Vec<Option<Token>>,
    op: Vec<Option<Box<Op>>>,
    value: Vec<i64>,
    /// Freed slots, oldest first.
    free: VecDeque<Id>,
    retired: usize,
    live_count: usize,
    /// Roots: every guard's node and its flag.
    roots: Vec<(Id, Liveness)>,
    epoch: u32,
    tx: u32,
    gray: Vec<Id>,
    order: Vec<Id>,
    stack: Vec<(Id, usize)>,
}

impl Arena {
    /// Slots in the arena, live or free.
    pub fn slots(&self) -> usize {
        self.live.len()
    }

    pub fn live_count(&self) -> usize {
        self.live_count
    }

    /// A slot for a new node: the oldest freed one, or a new one at the end.
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
        self.live_count += 1;
        self.deps[i as usize].extend_from_slice(deps);
        self.reach[i as usize].extend_from_slice(reach);
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

    /// Validates a token: graph id, liveness, generation.
    pub fn lookup(&self, t: Token) -> Option<Id> {
        let n = t.index as usize;
        let ok = t.graph == GRAPH
            && n < self.live.len()
            && self.live[n]
            && self.generation[n] == t.generation;
        ok.then_some(t.index)
    }

    /// Sets the token cell `i`'s committed value holds.
    pub fn hold_token(&mut self, i: Id, t: Token) {
        self.held[i as usize] = Some(t);
    }

    /// Roots node `i` for as long as the returned guard lives.
    pub fn guard(&mut self, i: Id) -> Guard {
        let flag = Rc::new(Cell::new(true));
        self.roots.push((i, Liveness(flag.clone())));
        Guard(flag)
    }

    /// Grows the scratch a collection and a transaction reuse, and the
    /// free list, to what the arena could need, as a runtime past its first
    /// collection has them. `Vec::clone` drops spare capacity, so a clone
    /// calls this again.
    pub fn reserve_scratch(&mut self) {
        let n = self.slots();
        self.gray.reserve(n);
        self.order.reserve(n);
        self.stack.reserve(n);
        self.free.reserve(n);
    }

    /// One collection: mark, sweep, prune. Returns how many it freed.
    pub fn collect(&mut self) -> usize {
        self.mark_roots();
        let freed = self.sweep();
        if freed > 0 {
            self.prune();
        }
        freed
    }

    /// Marks everything the live guards reach, through dependencies,
    /// recorded reach and held tokens, under a fresh visit stamp, and drops
    /// the released guards' entries. Returns how many nodes it reached.
    pub fn mark_roots(&mut self) -> usize {
        self.epoch += 1;
        let epoch = self.epoch;
        self.roots.retain(|(_, flag)| flag.0.get());
        let Arena {
            generation,
            live,
            visit,
            deps,
            reach,
            held,
            roots,
            gray,
            ..
        } = self;
        let mut shade = |gray: &mut Vec<Id>, i: Id| {
            let v = &mut visit[i as usize];
            if *v != epoch {
                *v = epoch;
                gray.push(i);
            }
        };
        for &(i, _) in roots.iter() {
            shade(gray, i);
        }
        let mut reached = 0;
        while let Some(n) = gray.pop() {
            reached += 1;
            let at = n as usize;
            for &d in &deps[at] {
                shade(gray, d);
            }
            for &r in &reach[at] {
                shade(gray, r);
            }
            // A token that fails its check names nothing.
            if let Some(t) = held[at] {
                let m = t.index as usize;
                if t.graph == GRAPH && m < live.len() && live[m] && generation[m] == t.generation {
                    shade(gray, t.index);
                }
            }
        }
        reached
    }

    /// Frees every live node the last mark did not reach, in index order:
    /// drops its payload and held token, clears its liveness, bumps its
    /// generation, and puts it on the back of the free list or retires it.
    /// Its lists keep their capacity for reuse. Returns how many it freed.
    pub fn sweep(&mut self) -> usize {
        let epoch = self.epoch;
        let mut freed = 0;
        for n in 0..self.live.len() {
            if self.live[n] && self.visit[n] != epoch {
                self.live[n] = false;
                self.op[n] = None;
                self.held[n] = None;
                self.generation[n] += 1;
                if self.generation[n] == u32::MAX {
                    self.retired += 1;
                } else {
                    self.free.push_back(n as Id);
                }
                freed += 1;
            }
        }
        self.live_count -= freed;
        freed
    }

    /// Takes the freed out of every survivor's dependents list, so a reused
    /// slot is never mistaken for the node it held. A survivor's
    /// dependencies and reach are live, since it reached them.
    pub fn prune(&mut self) {
        let Arena {
            live, dependents, ..
        } = self;
        for n in 0..live.len() {
            if live[n] {
                dependents[n].retain(|&d| live[d as usize]);
            }
        }
    }

    /// The per-slot floor: one pass over the arena that reads each slot's
    /// liveness and visit stamp once, what the sweep's test reads, and
    /// counts the live ones the last mark reached.
    pub fn touch_each_slot(&self) -> usize {
        let epoch = self.epoch;
        self.live
            .iter()
            .zip(&self.visit)
            .filter(|&(&l, &v)| l && v == epoch)
            .count()
    }

    /// One transaction: sends `x` to `input`, marks what depends on it
    /// depth first over dependents, and evaluates the reverse post-order.
    /// Returns the last value evaluated.
    pub fn transaction(&mut self, input: Id, x: i64) -> i64 {
        self.tx += 1;
        let tx = self.tx;
        self.order.clear();
        self.stack.clear();
        self.mark[input as usize] = tx;
        self.stack.push((input, 0));
        while let Some(&mut (n, ref mut k)) = self.stack.last_mut() {
            let ds = &self.dependents[n as usize];
            if *k < ds.len() {
                let d = ds[*k];
                *k += 1;
                if self.mark[d as usize] != tx {
                    self.mark[d as usize] = tx;
                    self.stack.push((d, 0));
                }
            } else {
                self.order.push(n);
                self.stack.pop();
            }
        }
        let mut last = 0;
        for k in (0..self.order.len()).rev() {
            let n = self.order[k] as usize;
            let op = **self.op[n].as_ref().expect("a marked node is live");
            let v = match op {
                Op::Input => x,
                Op::Map(c) => self.deps[n]
                    .iter()
                    .fold(c, |a, &d| a.wrapping_add(self.value[d as usize])),
                Op::Hold => self.value[self.deps[n][0] as usize],
                Op::Snapshot => self.value[self.deps[n][0] as usize]
                    .wrapping_add(self.value[self.reach[n][0] as usize]),
            };
            self.value[n] = v;
            last = v;
        }
        last
    }
}

/// A small fixed-seed generator, SplitMix64, so every run builds the same
/// graph.
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

/// A screen's input, which a transaction fires, its last node, which its
/// listener roots, and its size.
#[derive(Clone, Copy, Debug)]
pub struct Screen {
    pub input: Id,
    pub last: Id,
    pub nodes: u32,
}

/// An arena filled with screens, with the dropped screens' listeners
/// released and nothing freed yet: the state a collection starts from.
/// Cloning it gives every measured collection the same start.
pub struct Fixture {
    pub arena: Arena,
    /// A kept screen, the one the baseline transaction fires.
    pub kept: Screen,
    /// Nodes the collection will leave live.
    pub survivors: usize,
    /// The kept guards, shared by the fixture's clones.
    guards: Rc<Vec<Guard>>,
}

impl Clone for Fixture {
    fn clone(&self) -> Fixture {
        let mut arena = self.arena.clone();
        arena.reserve_scratch();
        Fixture {
            arena,
            guards: self.guards.clone(),
            ..*self
        }
    }
}

impl Fixture {
    /// Fills an arena with screens until it has at least `slots` slots,
    /// keeps the listeners of screens chosen in a shuffled order until at
    /// least `live_percent` of the slots will survive, and drops the rest.
    pub fn new(slots: usize, live_percent: usize) -> Fixture {
        let mut rng = Rng(0x5EED_0003);
        let mut a = Arena::default();
        let mut guards = Vec::new();
        // The core the application anchors.
        let clock = a.alloc(Op::Input, &[], &[]);
        let theme = a.alloc(Op::Hold, &[clock], &[]);
        let locale = a.alloc(Op::Hold, &[clock], &[]);
        guards.push(a.guard(theme));
        guards.push(a.guard(locale));
        // The screen the baseline transaction fires, the same shape in
        // every fixture, and always kept.
        let first = screen(&mut a, &mut Rng(0xBA5E), theme, locale);
        guards.push(a.guard(first.last));
        let mut screens = Vec::new();
        while a.slots() < slots {
            screens.push(screen(&mut a, &mut rng, theme, locale));
        }
        for k in (1..screens.len()).rev() {
            let j = (rng.next() % (k as u64 + 1)) as usize;
            screens.swap(k, j);
        }
        let want = a.slots() * live_percent / 100;
        let mut survivors = 3 + first.nodes as usize;
        let listeners: Vec<Guard> = screens.iter().map(|s| a.guard(s.last)).collect();
        for (s, g) in screens.iter().zip(listeners) {
            if survivors < want {
                survivors += s.nodes as usize;
                guards.push(g);
            }
            // Otherwise `g` drops here, and the screen's listener with it.
        }
        a.reserve_scratch();
        Fixture {
            arena: a,
            kept: first,
            survivors,
            guards: Rc::new(guards),
        }
    }

    /// The fixture with its collection's mark done, where a measured sweep
    /// starts.
    pub fn marked(mut self) -> Fixture {
        self.arena.mark_roots();
        self
    }

    /// The fixture with its collection's mark and sweep done, where a
    /// measured prune starts.
    pub fn swept(mut self) -> Fixture {
        self.arena.mark_roots();
        self.arena.sweep();
        self
    }

    /// The fixture after its collection and one transaction, where a
    /// measured transaction starts.
    pub fn collected(mut self) -> Fixture {
        self.arena.collect();
        self.arena.transaction(self.kept.input, 1);
        self
    }

    /// Guards the fixture holds: the core's and the kept screens'.
    pub fn guards(&self) -> usize {
        self.guards.len()
    }
}

/// Builds one screen reading the core's two cells.
fn screen(a: &mut Arena, rng: &mut Rng, theme: Id, locale: Id) -> Screen {
    let first = a.slots();
    let input = a.alloc(Op::Input, &[], &[]);
    let mut ends = Vec::new();
    for b in 0..rng.range(2, 6) {
        // The first branch also reads the theme, so the core's dependents
        // lists name every screen, as an app-wide cell's would.
        let mut at = if b == 0 {
            a.alloc(Op::Map(1), &[input, theme], &[])
        } else {
            a.alloc(Op::Map(i64::from(b)), &[input], &[])
        };
        for c in 0..rng.range(1, 8) {
            at = a.alloc(Op::Map(i64::from(c)), &[at], &[]);
        }
        ends.push(at);
    }
    let merge = a.alloc(Op::Map(0), &ends, &[]);
    let hold = a.alloc(Op::Hold, &[merge], &[]);
    let snap = a.alloc(Op::Snapshot, &[input], &[hold]);
    let select = a.alloc(Op::Hold, &[snap], &[]);
    // Reached only through the token `select` holds, and depends on
    // `select`: a cycle through values.
    let detail = a.alloc(Op::Map(0), &[select, locale], &[]);
    let t = a.token(detail);
    a.hold_token(select, t);
    let last = a.alloc(Op::Map(0), &[hold, select], &[]);
    Screen {
        input,
        last,
        nodes: (a.slots() - first) as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A collection frees exactly the dropped screens, their tokens go
    /// stale, the kept screen still runs, and the core's dependents lists
    /// name only survivors. Prints each fixture's census.
    #[test]
    fn collection_frees_the_dropped_screens() {
        for slots in [1_000, 10_000, 100_000] {
            for live in [10, 90] {
                let mut f = Fixture::new(slots, live);
                let before = f.arena.live_count();
                let kept = f.arena.token(f.kept.last);
                let theme_before = f.arena.dependents[1].len();
                let freed = f.arena.collect();
                assert_eq!(f.arena.live_count(), f.survivors);
                assert_eq!(before - freed, f.survivors);
                assert_eq!(f.arena.lookup(kept), Some(f.kept.last));
                assert_eq!(f.arena.free.len(), freed);
                let dead = (0..f.arena.slots())
                    .find(|&n| !f.arena.live[n])
                    .expect("something was freed");
                let stale = Token {
                    generation: 0,
                    ..f.arena.token(dead as Id)
                };
                assert_eq!(f.arena.lookup(stale), None);
                let theme_after = f.arena.dependents[1].len();
                assert!(
                    f.arena.dependents[1]
                        .iter()
                        .all(|&d| f.arena.live[d as usize])
                );
                assert_ne!(f.arena.transaction(f.kept.input, 7), 0);
                eprintln!(
                    "{slots} at {live}%: {} slots, {} survive ({:.1}%), {} freed, {} guards, \
                     theme dependents {theme_before} -> {theme_after}, kept screen {} nodes",
                    f.arena.slots(),
                    f.survivors,
                    100.0 * f.survivors as f64 / f.arena.slots() as f64,
                    freed,
                    f.guards(),
                    f.kept.nodes,
                );
            }
        }
    }
}
