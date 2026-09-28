//! Does RFD 3's collection trigger need a work term: over a long run of
//! navigations and clicks, does pacing collection against the garbage
//! transactions evaluate cost less per transaction, collections included,
//! than collecting when allocations outgrow the last survivors, and what do
//! the policies' longest pauses cost?
//!
//! RFD 3 ("When Collection Runs") collects after a whole unit when the
//! nodes allocated plus the guards released since the last collection
//! exceed the count that collection left alive (spike F64). Garbage costs
//! time as well as memory, since it is marked and evaluated whenever its
//! inputs fire until it is collected (F66). `rfd_0005_demand_bounded_push`
//! found, by arithmetic from per-operation counts, that in an application
//! with ten thousand live nodes the trigger lets about four hundred dead
//! screens build up, each costing every transaction on its inputs, and
//! that pacing against that work would cost about a tenth as much. This
//! probe runs it end to end.
//!
//! # The model
//!
//! The graph, arena and collection are that probe's, through its public
//! module: F66's navigation shape (`nav`, about thirty live nodes) and the
//! `app` shape (430 screens kept live on the clock beside it, about 9,400
//! live nodes the clicks don't reach). A run is a sequence of units: every
//! third one a navigation (a screen built, the switch moved onto it, the
//! old screen left unreachable), the others a transaction on the clicks.
//! After each unit the policy decides whether to collect, and a collection
//! is the whole of RFD 3's: mark, prune the dead out of dependents lists,
//! sweep. No guard is released in this workload, since a navigation moves
//! a switch, so RFD 3's release term is always zero here.
//!
//! # The policies
//!
//! - `Baseline`: collect after every navigation. No transaction ever sees
//!   garbage: the floor for transactions, not for collections.
//! - `Rfd3`: RFD 3 as written, read as F64 reads it. Nothing is freed
//!   between collections, so the nodes allocated since the last one are
//!   the live count less its survivors, which is the counter an engine
//!   keeps anyway.
//! - `Excess`: RFD 3's term, or a work term: the nodes by which each
//!   transaction's region exceeds the region the same input had in its
//!   first transaction after the last collection, summed since then,
//!   exceed the survivors. The region's length is the mark's output,
//!   already in hand; the reference is two words on the input node (the
//!   collection it was taken after, and the length). A region can't hold
//!   garbage a collection has freed, so growth past the reference is dead
//!   nodes or live growth, and live growth costs at most one early
//!   collection, after which it is the reference. Weighing a region node
//!   against a survivor one to one is the rent-or-buy rule: collect once
//!   the garbage work paid since the last collection matches what a
//!   collection costs. When garbage grows at a steady rate, as here, that
//!   is the interval that minimizes the total.
//! - `Total`: RFD 3's term, or every region node since the last collection
//!   exceeding the survivors, with no reference. It needs no per-input
//!   state, and it collects with no garbage at all whenever transactions
//!   are large against the live set.

use crate::rfd_0005_demand_bounded_push::{Fixture, Id, Push, Refresh};

/// The measured window: 1,300 navigations and 2,600 clicks, three of
/// `Rfd3`'s cycles in `app`, where its garbage per click comes within 0.1%
/// of a 30,000-unit run's.
pub const WINDOW: usize = 3_900;

/// Screens kept live in the `app` shape.
pub const APP_KEPT: usize = 430;

/// When collection runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    /// After every navigation.
    Baseline,
    /// Allocated plus released since the last collection exceed its
    /// survivors.
    Rfd3,
    /// `Rfd3`, or the regions' growth past each input's reference since the
    /// last collection exceeds its survivors.
    Excess,
    /// `Rfd3`, or the regions' total since the last collection exceeds its
    /// survivors.
    Total,
}

pub const POLICIES: [Policy; 4] = [
    Policy::Baseline,
    Policy::Rfd3,
    Policy::Excess,
    Policy::Total,
];

impl Policy {
    pub fn name(self) -> &'static str {
        match self {
            Policy::Baseline => "baseline",
            Policy::Rfd3 => "rfd3",
            Policy::Excess => "excess",
            Policy::Total => "total",
        }
    }
}

/// The trigger's state: what an engine would keep beside the arena.
#[derive(Clone, Debug)]
pub struct Pacer {
    pub policy: Policy,
    /// The live count the last collection left.
    survivors: usize,
    /// Guards released since the last collection; always zero here.
    released: usize,
    /// Region nodes since the last collection, by the policy's measure.
    work: usize,
    /// Collections so far; a reference taken before the last is stale.
    epoch: u32,
    /// Per input: the epoch its reference was taken in, and the region's
    /// length then. In an engine, two words on the input node.
    reference: Vec<(u32, u32)>,
}

impl Pacer {
    pub fn new(policy: Policy, survivors: usize) -> Pacer {
        Pacer {
            policy,
            survivors,
            released: 0,
            work: 0,
            epoch: 1,
            reference: Vec::new(),
        }
    }

    /// The fast path: runs after every transaction on `input` whose region
    /// was `region` nodes long.
    #[inline]
    pub fn observe(&mut self, input: Id, region: usize) {
        match self.policy {
            Policy::Baseline | Policy::Rfd3 => {}
            Policy::Total => self.work += region,
            Policy::Excess => {
                let i = input as usize;
                if i >= self.reference.len() {
                    self.reference.resize(i + 1, (0, 0));
                }
                let r = &mut self.reference[i];
                if r.0 != self.epoch {
                    *r = (self.epoch, region as u32);
                }
                self.work += region.saturating_sub(r.1 as usize);
            }
        }
    }

    /// Whether to collect after a unit, given the live count now and
    /// whether the unit navigated.
    #[inline]
    pub fn due(&self, live: usize, navigated: bool) -> bool {
        let allocated = live - self.survivors;
        let rfd3 = allocated + self.released > self.survivors;
        match self.policy {
            Policy::Baseline => navigated,
            Policy::Rfd3 => rfd3,
            Policy::Excess | Policy::Total => rfd3 || self.work > self.survivors,
        }
    }

    /// Resets the counts after a collection that left `survivors` alive.
    pub fn collected(&mut self, survivors: usize) {
        self.survivors = survivors;
        self.released = 0;
        self.work = 0;
        self.epoch += 1;
    }
}

/// One collection a run made.
#[derive(Clone, Copy, Debug, Default)]
pub struct Collection {
    /// The unit it followed, counted from the start of the window.
    pub unit: usize,
    pub freed: usize,
    pub survivors: usize,
}

/// What a run has done since its window started.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub units: usize,
    pub clicks: usize,
    /// Region nodes over every click.
    pub region_nodes: usize,
    pub collections: usize,
    /// Nodes freed over every collection.
    pub freed: usize,
    pub largest: Collection,
    pub max_slots: usize,
    /// The sum of the switch's values, which every policy must agree on.
    pub checksum: i64,
}

/// A long run of navigations and clicks under one policy.
#[derive(Clone)]
pub struct Run {
    pub fixture: Fixture,
    pub pacer: Pacer,
    /// Units since the run began, which sets the schedule.
    unit: usize,
    pub stats: Stats,
}

impl Run {
    /// The core, `kept` screens kept live, one screen on show, collected.
    pub fn new(kept: usize, policy: Policy) -> Run {
        let fixture = Fixture::new(kept, 0, Refresh::Collect).warmed(Push::All);
        let pacer = Pacer::new(policy, fixture.arena.live_count());
        Run {
            fixture,
            pacer,
            unit: 0,
            stats: Stats::default(),
        }
    }

    /// Runs units until the policy has collected once, so that a measured
    /// window starts where the policy's own cycle does, then starts the
    /// window.
    pub fn warmed(mut self) -> Run {
        while self.stats.collections == 0 {
            self.unit();
        }
        self.stats = Stats::default();
        self.fixture.arena.reserve_scratch();
        self
    }

    /// One unit's work, without the trigger: a navigation every third unit,
    /// else a click. Returns whether it navigated.
    pub fn step(&mut self) -> bool {
        let k = self.unit;
        self.unit += 1;
        self.stats.units += 1;
        if k.is_multiple_of(3) {
            self.fixture.navigate();
            return true;
        }
        self.click_at(k)
    }

    /// A click unit's work, whatever the schedule says. Returns false, for
    /// `settle`: it didn't navigate.
    pub fn click(&mut self) -> bool {
        let k = self.unit;
        self.unit += 1;
        self.stats.units += 1;
        self.click_at(k)
    }

    fn click_at(&mut self, k: usize) -> bool {
        let v = self.fixture.click(k as i64, Push::All);
        let region = self.fixture.arena.region().len();
        self.pacer.observe(self.fixture.clicks, region);
        self.stats.clicks += 1;
        self.stats.region_nodes += region;
        self.stats.checksum = self.stats.checksum.wrapping_add(v);
        false
    }

    /// The trigger after a unit, and the collection if it is due. Returns
    /// how many nodes the collection freed, if one ran.
    pub fn settle(&mut self, navigated: bool) -> Option<usize> {
        let arena = &self.fixture.arena;
        self.stats.max_slots = self.stats.max_slots.max(arena.slots());
        if !self.pacer.due(arena.live_count(), navigated) {
            return None;
        }
        Some(self.collect())
    }

    /// Collects now, whatever the trigger says. Returns how many it freed.
    pub fn collect(&mut self) -> usize {
        let arena = &mut self.fixture.arena;
        let freed = arena.collect();
        let survivors = arena.live_count();
        self.pacer.collected(survivors);
        let s = &mut self.stats;
        s.collections += 1;
        s.freed += freed;
        if freed > s.largest.freed {
            s.largest = Collection {
                unit: s.units.saturating_sub(1),
                freed,
                survivors,
            };
        }
        freed
    }

    /// One whole unit, collection included.
    pub fn unit(&mut self) {
        let navigated = self.step();
        self.settle(navigated);
    }

    /// Runs `units` units. Returns the checksum.
    pub fn run(&mut self, units: usize) -> i64 {
        for _ in 0..units {
            self.unit();
        }
        self.stats.checksum
    }

    /// A warmed run's window of `units` units replayed up to its largest
    /// collection, the unit's work done and the collection not, so what
    /// remains is that collection: the policy's longest pause.
    pub fn before_largest(kept: usize, policy: Policy, units: usize) -> Run {
        let start = Run::new(kept, policy).warmed();
        let mut probe = start.clone();
        probe.run(units);
        let at = probe.stats.largest.unit;
        let mut run = start;
        while run.stats.units < at {
            run.unit();
        }
        run.step();
        run.fixture.arena.reserve_scratch();
        run
    }

    /// A run with no garbage, collected after a navigation, whose next unit
    /// is a click, under `policy`'s pacer with its reference taken: the
    /// fast path's cost is one unit of it.
    pub fn fast_path(kept: usize, policy: Policy) -> Run {
        let mut r = Run::new(kept, Policy::Baseline).warmed();
        r.pacer = Pacer::new(policy, r.fixture.arena.live_count());
        let navigated = r.click();
        r.settle(navigated);
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHAPES: [(&str, usize); 2] = [("nav", 0), ("app", APP_KEPT)];

    /// Every policy computes the same values and, collected at the end,
    /// leaves the same nodes alive.
    #[test]
    fn every_policy_computes_the_same_values() {
        for (_, kept) in SHAPES {
            let mut sums = Vec::new();
            let mut live = Vec::new();
            for p in POLICIES {
                let mut r = Run::new(kept, p);
                sums.push(r.run(1_500));
                r.collect();
                live.push(r.fixture.arena.live_count());
            }
            assert!(sums.iter().all(|&s| s == sums[0]), "{sums:?}");
            assert!(live.iter().all(|&n| n == live[0]), "{live:?}");
        }
    }

    /// With no garbage, a click's region equals its reference, so `excess`
    /// accrues nothing, and the replay stops right before the collection
    /// the full run called largest.
    #[test]
    fn excess_is_zero_without_garbage_and_the_replay_is_exact() {
        let mut r = Run::new(430, Policy::Excess);
        r.step();
        r.collect();
        r.step();
        r.step();
        assert_eq!(r.pacer.work, 0);
        let mut probe = Run::new(430, Policy::Rfd3).warmed();
        probe.run(3_000);
        let mut before = Run::before_largest(430, Policy::Rfd3, 3_000);
        assert_eq!(before.collect(), probe.stats.largest.freed);
    }

    /// The counts the note quotes: per shape and policy over a long run,
    /// collections, the clicks' regions, and the largest collection.
    #[test]
    fn counts() {
        const UNITS: usize = 30_000;
        println!(
            "{UNITS} units from each policy's first collection, every third a navigation, \
             the rest a click"
        );
        for (shape, kept) in SHAPES {
            let r = Run::new(kept, Policy::Baseline).warmed();
            let floor = r.fixture.arena.live_count();
            println!();
            println!(
                "{shape}: {floor} live after a collection, the screen on show {} nodes",
                r.fixture.current_nodes()
            );
            println!(
                "  {:<9} {:>11} {:>13} {:>13} {:>14} {:>14} {:>10}",
                "policy",
                "collections",
                "navs between",
                "region/click",
                "freed/collect",
                "largest freed",
                "max slots"
            );
            for p in POLICIES {
                let mut r = Run::new(kept, p).warmed();
                r.run(UNITS);
                let s = &r.stats;
                println!(
                    "  {:<9} {:>11} {:>13.1} {:>13.1} {:>14.1} {:>14} {:>10}",
                    p.name(),
                    s.collections,
                    (UNITS / 3) as f64 / s.collections as f64,
                    s.region_nodes as f64 / s.clicks as f64,
                    s.freed as f64 / s.collections as f64,
                    s.largest.freed,
                    s.max_slots,
                );
            }
        }
    }
}
