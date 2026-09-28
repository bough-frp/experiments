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
//!
//! # The uneven workload
//!
//! The first run had one input, so `Excess`'s per-input reference was never
//! tested where it could go wrong: an input whose first transaction after a
//! collection comes after some navigations takes their garbage into its
//! reference and never counts it (a missed collection), and a live region
//! that grows is counted as if it were garbage (a spurious one). [`Uneven`]
//! runs four inputs at uneven rates, one every unit, one every tenth, one
//! every hundredth, and one once early and then every 1,500th; a theme over
//! the rare one that every screen reads, so its region is nearly the whole
//! graph; a screen kept and anchored every thirtieth unit, off each frequent
//! input in turn, so their live regions grow about fivefold over the run;
//! and navigation, every third unit for 900 units of each 1,200 and none in
//! the last 300, so there are stretches with growth and no garbage. Its
//! screens hang off every frequent input in turn (`Mix::Spread`) or off the
//! tenth and hundredth only (`Mix::Sparse`), where no input that fires every
//! unit sees garbage.
//!
//! An audit, off in the benches, knows which nodes navigation left dead and
//! counts how many each transaction evaluates. That gives `Oracle`, the
//! rent-or-buy rule with the true dead-node work in place of an estimate;
//! a missed collection, a unit after which that work since the last
//! collection exceeds its survivors and no collection runs; and a spurious
//! one, which frees fewer nodes than the smallest screen has. Two fixes are
//! run beside `Excess`: `Decay`, a reference that outlives a collection so
//! garbage made before an input's first transaction still counts, and
//! `Backoff`, a threshold that grows after collections that freed little,
//! against live growth.

use std::rc::Rc;

use crate::rfd_0005_demand_bounded_push::{Arena, Fixture, Guard, Id, Op, Push, Refresh};

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
    /// `Excess` with a reference that outlives collections: a smaller region
    /// lowers it at once, and the first region after a collection raises it
    /// only halfway, so garbage made before an input's first transaction
    /// after a collection still counts, and live growth fades out.
    Decay,
    /// `Excess`, whose threshold doubles (up to 64 times the survivors)
    /// after a collection that freed under 1% of what it left alive, and
    /// falls back to the survivors after one that freed more.
    Backoff,
    /// Collects when the dead nodes transactions evaluated since the last
    /// collection exceed its survivors, or on `Rfd3`. An engine can't count
    /// them; only a run with an audit can, so it is the target, not a
    /// candidate.
    Oracle,
}

pub const POLICIES: [Policy; 4] = [
    Policy::Baseline,
    Policy::Rfd3,
    Policy::Excess,
    Policy::Total,
];

/// The policies the uneven workload runs; `Oracle` only in the counts.
pub const UNEVEN_POLICIES: [Policy; 6] = [
    Policy::Baseline,
    Policy::Rfd3,
    Policy::Excess,
    Policy::Total,
    Policy::Decay,
    Policy::Backoff,
];

impl Policy {
    pub fn name(self) -> &'static str {
        match self {
            Policy::Baseline => "baseline",
            Policy::Rfd3 => "rfd3",
            Policy::Excess => "excess",
            Policy::Total => "total",
            Policy::Decay => "decay",
            Policy::Backoff => "backoff",
            Policy::Oracle => "oracle",
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
    /// `Backoff`'s multiple of the survivors the work must exceed.
    scale: usize,
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
            scale: 1,
        }
    }

    /// The fast path: runs after every transaction on `input` whose region
    /// was `region` nodes long.
    #[inline]
    pub fn observe(&mut self, input: Id, region: usize) {
        match self.policy {
            Policy::Baseline | Policy::Rfd3 | Policy::Oracle => {}
            Policy::Total => self.work += region,
            Policy::Excess | Policy::Backoff => {
                let epoch = self.epoch;
                let r = self.reference(input);
                if r.0 != epoch {
                    *r = (epoch, region as u32);
                }
                let at = r.1 as usize;
                self.work += region.saturating_sub(at);
            }
            Policy::Decay => {
                let epoch = self.epoch;
                let r = self.reference(input);
                let len = region as u32;
                if r.0 == 0 {
                    *r = (epoch, len);
                } else if r.0 != epoch {
                    // A collection since: whatever the region lost was
                    // garbage, and what it gained is live growth or garbage
                    // made since, which only a later collection can tell
                    // apart, so the reference goes halfway.
                    r.0 = epoch;
                    r.1 = if len <= r.1 {
                        len
                    } else {
                        r.1 + (len - r.1) / 2
                    };
                } else if len < r.1 {
                    r.1 = len;
                }
                let at = r.1 as usize;
                self.work += region.saturating_sub(at);
            }
        }
    }

    /// `Oracle`'s count: `dead` of the last transaction's region nodes were
    /// unreachable, which only an audit knows.
    #[inline]
    pub fn observe_dead(&mut self, dead: usize) {
        if self.policy == Policy::Oracle {
            self.work += dead;
        }
    }

    fn reference(&mut self, input: Id) -> &mut (u32, u32) {
        let i = input as usize;
        if i >= self.reference.len() {
            self.reference.resize(i + 1, (0, 0));
        }
        &mut self.reference[i]
    }

    /// The live count the last collection left.
    pub fn survivors(&self) -> usize {
        self.survivors
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
            Policy::Excess | Policy::Total | Policy::Decay | Policy::Oracle => {
                rfd3 || self.work > self.survivors
            }
            Policy::Backoff => rfd3 || self.work > self.survivors * self.scale,
        }
    }

    /// Resets the counts after a collection that freed `freed` and left
    /// `survivors` alive.
    pub fn collected(&mut self, survivors: usize, freed: usize) {
        if self.policy == Policy::Backoff {
            self.scale = if freed * 100 < survivors {
                (self.scale * 2).min(64)
            } else {
                1
            };
        }
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
        self.pacer.collected(survivors, freed);
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

/// Units the uneven workload runs under its policy before its window: two
/// periods, so that every policy has settled into its own cycle and every
/// window starts at the same point of the schedule.
pub const UNEVEN_WARMUP: usize = 2_400;

/// The uneven workload's measured window: three periods, each 900 units
/// navigating every third unit and 300 quiet ones.
pub const UNEVEN_WINDOW: usize = 3_600;

/// The schedule repeats every period: navigation for its first
/// `NAVIGATING` units, then none.
const PERIOD: usize = 1_200;
const NAVIGATING: usize = 900;

/// A screen is kept and anchored every `GROW_EVERY` units, whatever the
/// phase, round the three frequent inputs.
const GROW_EVERY: usize = 30;

/// The uneven workload's inputs, by how often they fire.
pub const INPUTS: [&str; 4] = ["every", "tenth", "hundredth", "rare"];

/// Screens kept live off each input at the start: a few off each frequent
/// one, and the bulk off the rare one, as `app`'s were off the clock.
const KEPT: [usize; 4] = [20, 20, 20, 370];

/// A collection that frees fewer nodes than the smallest screen has freed
/// less than one navigation's garbage: spurious.
pub const SPURIOUS: usize = 18;

/// Whether input `i` fires in unit `k`: every unit, every tenth, every
/// hundredth, and once early then every 1,500th (twice in the window).
fn fires(i: usize, k: usize) -> bool {
    match i {
        0 => true,
        1 => k % 10 == 7,
        2 => k % 100 == 53,
        _ => k == 5 || k % 1_500 == 700,
    }
}

/// Which inputs navigation hangs its screens off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mix {
    /// The three frequent inputs in turn, so most garbage is on an input
    /// that fires every unit.
    Spread,
    /// The tenth and the hundredth in turn: no garbage on an input that
    /// fires every unit, so an input's first transaction after a
    /// collection has usually seen some navigations first.
    Sparse,
}

pub const MIXES: [Mix; 2] = [Mix::Spread, Mix::Sparse];

impl Mix {
    pub fn name(self) -> &'static str {
        match self {
            Mix::Spread => "spread",
            Mix::Sparse => "sparse",
        }
    }
}

/// SplitMix64, with its own seed, so every policy builds the same screens.
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

/// `rfd_0005_demand_bounded_push`'s screen, 18 to 30 nodes off `src`, one
/// branch also off `side` and one off `theme`, returning every node it
/// built, its last last, so the audit knows what a navigation leaves dead.
fn screen(a: &mut Arena, rng: &mut Rng, src: Id, side: Id, theme: Id) -> Vec<Id> {
    let mut ids = Vec::with_capacity(30);
    let mut alloc = |a: &mut Arena, op: Op, deps: &[Id], reach: &[Id]| {
        let i = a.alloc(op, deps, reach);
        ids.push(i);
        i
    };
    let mut ends = Vec::new();
    for b in 0..rng.range(3, 4) {
        let mut at = match b {
            0 => alloc(a, Op::Map(1), &[src, theme], &[]),
            1 => alloc(a, Op::Map(2), &[src, side], &[]),
            _ => alloc(a, Op::Map(i64::from(b)), &[src], &[]),
        };
        for c in 0..rng.range(3, 5) {
            at = alloc(a, Op::Map(i64::from(c)), &[at], &[]);
        }
        ends.push(at);
    }
    let merge = alloc(a, Op::Map(0), &ends, &[]);
    let count = alloc(a, Op::Hold, &[merge], &[]);
    let snap = alloc(a, Op::Snapshot, &[src], &[count]);
    let select = alloc(a, Op::Hold, &[snap], &[]);
    let detail = alloc(a, Op::Map(0), &[select, theme], &[]);
    alloc(a, Op::Map(0), &[count, select], &[]);
    let t = a.token(detail);
    a.hold_token(select, t);
    ids
}

/// What the audit keeps: which nodes are dead, known because the workload
/// made them so. An engine has none of this.
#[derive(Clone, Default)]
struct Audit {
    dead: Vec<bool>,
    /// The dead since the last collection, which it must free exactly.
    pending: Vec<Id>,
    /// Dead nodes transactions evaluated since the last collection.
    since: usize,
}

/// What an uneven run has done since its window started.
#[derive(Clone, Debug, Default)]
pub struct UnevenStats {
    pub units: usize,
    /// Per input: transactions, their region nodes, and (audited) how many
    /// of those were dead.
    pub fires: [usize; 4],
    pub region: [usize; 4],
    pub dead: [usize; 4],
    pub collections: usize,
    pub freed: usize,
    /// Collections that freed fewer than `SPURIOUS` nodes.
    pub spurious: usize,
    /// Units after which the dead nodes evaluated since the last collection
    /// exceeded its survivors and no collection ran (audited).
    pub missed: usize,
    /// The largest ratio of those dead nodes to the survivors after a unit.
    pub peak: f64,
    /// Over every collection, the survivors it marked and the slots it
    /// swept: the model of what collecting cost.
    pub marked: usize,
    pub swept: usize,
    pub largest: Collection,
    pub max_slots: usize,
    /// The largest region total in one unit, a transaction's own pause.
    pub max_unit_region: usize,
    pub checksum: i64,
}

/// A long run of the uneven workload under one policy: four inputs firing
/// at uneven rates, screens kept off them so their live regions grow, and
/// navigation leaving garbage off all but the rare one.
pub struct Uneven {
    pub arena: Arena,
    mix: Mix,
    inputs: [Id; 4],
    theme: Id,
    nav: Id,
    switch: Id,
    /// The nodes of the screen on show, its last last.
    current: Vec<Id>,
    navigations: usize,
    grown: usize,
    guards: Vec<Rc<Guard>>,
    rng: Rng,
    pub pacer: Pacer,
    unit: usize,
    audit: Option<Audit>,
    pub stats: UnevenStats,
}

impl Clone for Uneven {
    fn clone(&self) -> Uneven {
        let mut arena = self.arena.clone();
        arena.reserve_scratch();
        Uneven {
            arena,
            current: self.current.clone(),
            guards: self.guards.clone(),
            rng: self.rng.clone(),
            pacer: self.pacer.clone(),
            audit: self.audit.clone(),
            stats: self.stats.clone(),
            ..*self
        }
    }
}

impl Uneven {
    /// The core, the kept screens, one screen on show, collected. With
    /// `audit`, the run counts the dead nodes its transactions evaluate,
    /// which costs a pass over every region.
    pub fn new(mix: Mix, policy: Policy, audit: bool) -> Uneven {
        assert!(
            audit || policy != Policy::Oracle,
            "the oracle needs an audit"
        );
        let mut a = Arena::default();
        let mut guards = Vec::new();
        let inputs = [0; 4].map(|_| a.alloc(Op::Input, &[], &[]));
        let navigate = a.alloc(Op::Input, &[], &[]);
        // A theme changes rarely, and every screen reads it.
        let theme = a.alloc(Op::Hold, &[inputs[3]], &[]);
        let nav = a.alloc(Op::Hold, &[navigate], &[]);
        let switch = a.alloc(Op::Switch, &[nav], &[]);
        for i in inputs.into_iter().chain([navigate, switch]) {
            guards.push(Rc::new(a.guard(i)));
        }
        let mut rng = Rng(0x5EED_0003);
        for (i, &n) in KEPT.iter().enumerate() {
            let side = if i == 3 {
                inputs[3]
            } else {
                inputs[(i + 1) % 3]
            };
            for _ in 0..n {
                let s = screen(&mut a, &mut rng, inputs[i], side, theme);
                guards.push(Rc::new(a.guard(*s.last().unwrap())));
            }
        }
        let mut u = Uneven {
            arena: a,
            mix,
            inputs,
            theme,
            nav,
            switch,
            current: Vec::new(),
            navigations: 0,
            grown: 0,
            guards,
            rng,
            pacer: Pacer::new(policy, 0),
            unit: 0,
            audit: audit.then(Audit::default),
            stats: UnevenStats::default(),
        };
        u.navigate();
        u.arena.collect();
        u.pacer = Pacer::new(policy, u.arena.live_count());
        if let Some(a) = &mut u.audit {
            a.pending.clear();
        }
        u.arena.reserve_scratch();
        u
    }

    /// Runs `UNEVEN_WARMUP` units under the policy, then starts the window.
    pub fn warmed(mut self) -> Uneven {
        self.run(UNEVEN_WARMUP);
        self.stats = UnevenStats::default();
        self.arena.reserve_scratch();
        self
    }

    /// A navigation: a screen off the mix's inputs, in turn, the switch
    /// moved onto it, the old one left unreachable.
    fn navigate(&mut self) {
        let n = self.navigations;
        self.navigations += 1;
        let (src, side) = match self.mix {
            Mix::Spread => (self.inputs[n % 3], self.inputs[(n + 1) % 3]),
            Mix::Sparse => (self.inputs[1 + n % 2], self.inputs[1 + (n + 1) % 2]),
        };
        let ids = screen(&mut self.arena, &mut self.rng, src, side, self.theme);
        let last = *ids.last().unwrap();
        let old = self.current.last().copied();
        self.arena.relink(self.switch, old, last);
        let t = self.arena.token(last);
        self.arena.hold_token(self.nav, t);
        let old = std::mem::replace(&mut self.current, ids);
        if let Some(a) = &mut self.audit {
            a.dead.resize(self.arena.slots(), false);
            for &i in &old {
                a.dead[i as usize] = true;
            }
            a.pending.extend(old);
        }
    }

    /// A screen kept and anchored off one of the three frequent inputs, in
    /// turn: its input's live region grows.
    fn grow(&mut self) {
        let g = self.grown;
        self.grown += 1;
        let (src, side) = (self.inputs[g % 3], self.inputs[(g + 2) % 3]);
        let ids = screen(&mut self.arena, &mut self.rng, src, side, self.theme);
        let last = *ids.last().unwrap();
        self.guards.push(Rc::new(self.arena.guard(last)));
    }

    fn fire(&mut self, i: usize, k: usize) -> usize {
        let input = self.inputs[i];
        self.arena.transaction(input, k as i64, Push::All);
        let region = self.arena.region().len();
        self.pacer.observe(input, region);
        let s = &mut self.stats;
        s.fires[i] += 1;
        s.region[i] += region;
        s.checksum = s.checksum.wrapping_add(self.arena.value(self.switch));
        if let Some(a) = &mut self.audit {
            let dead = self
                .arena
                .region()
                .iter()
                .filter(|&&n| a.dead.get(n as usize) == Some(&true))
                .count();
            a.since += dead;
            s.dead[i] += dead;
            self.pacer.observe_dead(dead);
        }
        region
    }

    /// One unit's work, without the trigger: a navigation every third unit
    /// in a period's first 900, a kept screen every 30th, then each input
    /// that fires. Returns whether it navigated.
    pub fn step(&mut self) -> bool {
        let k = self.unit;
        self.unit += 1;
        self.stats.units += 1;
        let navigated = k % PERIOD < NAVIGATING && k.is_multiple_of(3);
        if navigated {
            self.navigate();
        }
        if k % GROW_EVERY == GROW_EVERY / 2 {
            self.grow();
        }
        let mut region = 0;
        for i in 0..4 {
            if fires(i, k) {
                region += self.fire(i, k);
            }
        }
        self.stats.max_unit_region = self.stats.max_unit_region.max(region);
        navigated
    }

    /// The trigger after a unit, and the collection if it is due. Returns
    /// how many nodes the collection freed, if one ran.
    pub fn settle(&mut self, navigated: bool) -> Option<usize> {
        let slots = self.arena.slots();
        self.stats.max_slots = self.stats.max_slots.max(slots);
        let due = self.pacer.due(self.arena.live_count(), navigated);
        if let Some(a) = &self.audit {
            let survivors = self.pacer.survivors();
            let s = &mut self.stats;
            s.peak = s.peak.max(a.since as f64 / survivors as f64);
            if !due && a.since > survivors {
                s.missed += 1;
            }
        }
        due.then(|| self.collect())
    }

    /// Collects now, whatever the trigger says. Returns how many it freed.
    pub fn collect(&mut self) -> usize {
        let slots = self.arena.slots();
        let freed = self.arena.collect();
        let survivors = self.arena.live_count();
        self.pacer.collected(survivors, freed);
        if let Some(a) = &mut self.audit {
            assert_eq!(freed, a.pending.len(), "a collection frees the dead");
            for i in a.pending.drain(..) {
                a.dead[i as usize] = false;
            }
            a.since = 0;
        }
        let s = &mut self.stats;
        s.collections += 1;
        s.freed += freed;
        s.spurious += usize::from(freed < SPURIOUS);
        s.marked += survivors;
        s.swept += slots;
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
    /// collection, the unit's work done and the collection not.
    pub fn before_largest(mix: Mix, policy: Policy, units: usize) -> Uneven {
        let start = Uneven::new(mix, policy, false).warmed();
        let mut probe = start.clone();
        probe.run(units);
        let at = probe.stats.largest.unit;
        let mut run = start;
        while run.stats.units < at {
            run.unit();
        }
        run.step();
        run.arena.reserve_scratch();
        run
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

    /// Under the uneven workload every policy computes the same values and,
    /// collected at the end, leaves the same nodes alive; every collection
    /// frees exactly the screens navigation abandoned (the audit asserts it).
    #[test]
    fn every_policy_agrees_under_the_uneven_workload() {
        for mix in MIXES {
            let mut sums = Vec::new();
            let mut live = Vec::new();
            for p in UNEVEN_POLICIES.into_iter().chain([Policy::Oracle]) {
                let mut r = Uneven::new(mix, p, true);
                sums.push(r.run(1_600));
                r.collect();
                live.push(r.arena.live_count());
            }
            assert!(sums.iter().all(|&s| s == sums[0]), "{sums:?}");
            assert!(live.iter().all(|&n| n == live[0]), "{live:?}");
        }
    }

    /// The counts the note quotes for the uneven workload: per policy over
    /// its window, collections, spurious and missed ones, what collecting
    /// and the regions cost in node visits, and the largest collection;
    /// then where the regions' dead nodes were, per input.
    #[test]
    fn uneven_counts() {
        let start = Uneven::new(Mix::Spread, Policy::Baseline, true);
        println!(
            "uneven: {} live after the first collection; {UNEVEN_WINDOW} units after \
             {UNEVEN_WARMUP} of warm-up, in periods of {PERIOD}: {NAVIGATING} navigating every \
             third unit, then quiet; a screen kept every {GROW_EVERY} units",
            start.arena.live_count()
        );
        println!(
            "inputs fire every unit, every 10th, every 100th, and at unit 5 then every 1,500th; \
             spurious = freed < {SPURIOUS} (under one screen); missed = a unit after which the \
             dead nodes evaluated since the last collection exceeded its survivors, uncollected"
        );
        println!(
            "columns: peak = largest dead-evaluated/survivors after a unit; region/u, coll/u = \
             region nodes and collection visits (survivors marked + slots swept) per unit; \
             surv = survivors at the window's last collection"
        );
        for mix in MIXES {
            println!();
            println!(
                "{}: navigation off {}",
                mix.name(),
                match mix {
                    Mix::Spread => "every, tenth and hundredth in turn",
                    Mix::Sparse => "tenth and hundredth in turn",
                }
            );
            println!(
                "  {:<9} {:>6} {:>5} {:>6} {:>6} {:>8} {:>8} {:>8} {:>8} {:>8} {:>7} {:>7}",
                "policy",
                "colls",
                "spur",
                "missed",
                "peak",
                "region/u",
                "coll/u",
                "visits/u",
                "freed/c",
                "largest",
                "surv",
                "slots"
            );
            let mut runs = Vec::new();
            for p in UNEVEN_POLICIES.into_iter().chain([Policy::Oracle]) {
                let mut r = Uneven::new(mix, p, true).warmed();
                r.run(UNEVEN_WINDOW);
                let s = &r.stats;
                let u = s.units as f64;
                let region: usize = s.region.iter().sum();
                let coll = s.marked + s.swept;
                println!(
                    "  {:<9} {:>6} {:>5} {:>6} {:>6.2} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8} {:>7} \
                     {:>7}",
                    p.name(),
                    s.collections,
                    s.spurious,
                    s.missed,
                    s.peak,
                    region as f64 / u,
                    coll as f64 / u,
                    (region + coll) as f64 / u,
                    s.freed as f64 / s.collections.max(1) as f64,
                    s.largest.freed,
                    r.pacer.survivors(),
                    s.max_slots,
                );
                runs.push((p, r));
            }
            println!("  per input, region nodes / dead nodes per transaction");
            print!("  {:<9}", "policy");
            for (i, name) in INPUTS.iter().enumerate() {
                print!(" {:>17}", format!("{name} ({})", runs[0].1.stats.fires[i]));
            }
            println!(" {:>9}", "max unit");
            for (p, r) in &runs {
                let s = &r.stats;
                print!("  {:<9}", p.name());
                for i in 0..4 {
                    let f = s.fires[i].max(1) as f64;
                    print!(
                        " {:>17}",
                        format!(
                            "{:.0} / {:.0}",
                            s.region[i] as f64 / f,
                            s.dead[i] as f64 / f
                        )
                    );
                }
                println!(" {:>9}", s.max_unit_region);
            }
        }
    }
}
