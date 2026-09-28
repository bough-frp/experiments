//! Does a maintained topological order bound a switch move's same-instant
//! cycle check well below the unbounded upstream walk, on a 10,000-node
//! UI-shaped graph? And do per-subgraph dependency summaries?
//!
//! RFD 5 moves every switch at commit and then checks the final graph for a
//! same-instant cycle. The spike walks everything upstream of each new
//! inner, about 9 ns a node (F50). Checking the final graph rather than one
//! move at a time is what lets two switches reverse a dependency between
//! them in one instant (F46). Three checkers answer the same question here,
//! on the same workloads, and must agree on every transaction's verdict:
//!
//! - [`Baseline`]: every move applied, then a walk upstream from each new
//!   inner looking for its switch, the spike's check.
//! - [`Pk`]: a `u32` position per node kept as a topological order,
//!   Pearce and Kelly's algorithm. All of a transaction's unlinks go first,
//!   since a deletion never invalidates an order, and after them every
//!   intermediate graph is a subgraph of the final one, so F46's reversal
//!   is never refused. Then each new link inner → switch costs one
//!   comparison if the inner is already before the switch, and otherwise
//!   a forward search from the switch and a backward search from the inner,
//!   both inside the affected region between their two positions, and a
//!   reorder of what they found. A node built appends at the end.
//! - [`Summaries`]: each `construct`-style subgraph keeps, for every node
//!   in it, a bitset of the subgraph's *sources* that reach it inside the
//!   subgraph, after Pouzet and Raymond's input/output summaries. A source
//!   is a node with a dependency outside the subgraph, or a switch. The
//!   check walks from the new inner through sources and across subgraph
//!   boundaries instead of through every node. Every switch is a source,
//!   so a move whose inners are in other subgraphs changes no summary; a
//!   move with an inner inside the switch's own subgraph makes that
//!   subgraph's summary stale, and it is rebuilt at commit, which also
//!   finds a cycle local to it. A subgraph built during the instant gets
//!   its summary as it is built.
//!
//! The graph is a model of the UI shape, not the spike's engine: inputs, a
//! root event stream (a forward token closed over the root component's
//! events), application state held from it and lifted, and a tree of
//! components. Each component lifts its props and the state, derives
//! events from inputs, holds one of them, combines its children's views
//! into its view and merges their events into its events. A navigation
//! slot is a pair of switches over the same candidate screens: a
//! `switch_cell` over their views and a `switch_stream` over their events.
//! So the upstream of a view inner is the state, and through the root event
//! stream every linked component's events: much of the graph. The upstream
//! of an event inner is small, but the downstream of its switch is the
//! state and everything that reads it. Some components also have a local
//! switch over their own lifts.
//!
//! A transaction navigates a few slots. Each target is a screen built
//! during the instant, a reversal to the slot's previous screen, or another
//! of its existing screens, in tunable proportions. Some transactions also
//! move a local switch, and a few try a move that closes a cycle; that
//! transaction is refused and rolled back (the engine would poison instead,
//! but the workload has to go on). Every workload also carries F46's
//! reversal, once in each direction.

/// A node's index.
pub type Id = u32;

/// A tiny deterministic generator (SplitMix64), so the workloads are the
/// same on every run without a dependency.
#[derive(Clone)]
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n`.
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    /// Uniform in `lo..=hi`.
    fn range(&mut self, lo: usize, hi: usize) -> usize {
        lo + self.below(hi - lo + 1)
    }

    fn chance(&mut self, p: f64) -> bool {
        ((self.next() >> 11) as f64 / (1u64 << 53) as f64) < p
    }

    fn pick(&mut self, from: &[Id]) -> Id {
        from[self.below(from.len())]
    }
}

/// The dependency graph: an edge `a → b` means `b` depends on `a` at the
/// same instant. A switch has exactly one dependency, its inner.
#[derive(Clone, Default)]
pub struct Graph {
    deps: Vec<Vec<Id>>,
    dependents: Vec<Vec<Id>>,
    /// The `construct`-style subgraph each node was built in.
    sub: Vec<u32>,
    switch: Vec<bool>,
    subs: u32,
}

impl Graph {
    pub fn len(&self) -> usize {
        self.deps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.deps.is_empty()
    }

    fn add(&mut self, sub: u32, deps: &[Id], switch: bool) -> Id {
        let n = self.deps.len() as Id;
        for &d in deps {
            self.dependents[d as usize].push(n);
        }
        self.deps.push(deps.to_vec());
        self.dependents.push(Vec::new());
        self.sub.push(sub);
        self.switch.push(switch);
        self.subs = self.subs.max(sub + 1);
        n
    }

    fn link(&mut self, from: Id, to: Id) {
        self.deps[to as usize].push(from);
        self.dependents[from as usize].push(to);
    }

    fn unlink(&mut self, from: Id, to: Id) {
        let deps = &mut self.deps[to as usize];
        let k = deps.iter().position(|&d| d == from).expect("no such edge");
        deps.swap_remove(k);
        let out = &mut self.dependents[from as usize];
        let k = out.iter().position(|&d| d == to).expect("no such edge");
        out.swap_remove(k);
    }

    /// Whether `target` is upstream of `from`, walking dependencies: the
    /// spike's check, with a reused stack and a visit stamp. Adds the nodes
    /// it visits to `visited`.
    fn upstream(&self, from: Id, target: Id, walk: &mut Walk, visited: &mut u64) -> bool {
        walk.begin(self.len());
        walk.stack.push(from);
        walk.mark(from);
        let mut count = 0;
        let mut found = false;
        while let Some(n) = walk.stack.pop() {
            count += 1;
            if n == target {
                found = true;
                break;
            }
            for &d in &self.deps[n as usize] {
                if !walk.seen(d) {
                    walk.mark(d);
                    walk.stack.push(d);
                }
            }
        }
        *visited += count;
        found
    }
}

/// A reusable depth-first search: a stack and a per-node stamp, so nothing
/// is cleared between searches.
#[derive(Clone, Default)]
struct Walk {
    stack: Vec<Id>,
    stamp: Vec<u32>,
    epoch: u32,
}

impl Walk {
    fn begin(&mut self, len: usize) {
        self.stamp.resize(len, 0);
        self.epoch += 1;
        self.stack.clear();
    }

    fn mark(&mut self, n: Id) {
        self.stamp[n as usize] = self.epoch;
    }

    fn seen(&self, n: Id) -> bool {
        self.stamp[n as usize] == self.epoch
    }
}

/// What a move is, for the breakdown of the counts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// A `switch_cell` over a slot's screen views.
    View,
    /// A `switch_stream` over a slot's screen events.
    Event,
    /// A switch over lifts in its own subgraph.
    Local,
    /// A view move retargeted to close a cycle.
    Bad,
    /// One of F46's two switches.
    F46,
}

pub const KINDS: [Kind; 5] = [Kind::View, Kind::Event, Kind::Local, Kind::Bad, Kind::F46];

/// A switch moving from one inner to another at commit.
#[derive(Clone, Copy, Debug)]
pub struct Move {
    pub switch: Id,
    pub from: Id,
    pub to: Id,
    pub kind: Kind,
}

/// Nodes built during the instant, as one new subgraph: each node's
/// dependencies and whether it is a switch. Their ids follow on from the
/// graph's length, in this order.
#[derive(Clone)]
pub struct Build {
    sub: u32,
    nodes: Vec<(Vec<Id>, bool)>,
}

impl Build {
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// One transaction: the subgraphs built during it, then its switch moves.
#[derive(Clone)]
pub struct Tx {
    pub builds: Vec<Build>,
    pub moves: Vec<Move>,
}

/// A workload's knobs. Probabilities are per navigated slot unless noted.
#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// Nodes in the graph before the first transaction, roughly.
    pub nodes: usize,
    pub txs: usize,
    /// Navigation slots moved per transaction (two switches each).
    pub slots_per_tx: usize,
    /// The target screen is built during the instant.
    pub p_new: f64,
    /// Otherwise, the target is the slot's previous screen.
    pub p_rev: f64,
    /// Per transaction: the first view move instead targets a node
    /// downstream of its switch.
    pub p_bad: f64,
    /// Per transaction: a local switch also moves.
    pub p_local: f64,
    pub seed: u64,
}

/// Transactions per workload, in every bench and in the counts.
pub const TXS: usize = 300;

/// The workloads every bench and the counts run: the same 10,000-node
/// graph, with screens built eagerly (`settled`), sometimes during the
/// instant (`mixed`, `churn`), or always (`lazy`, the
/// `switch_cell(selector.map(build))` form).
pub fn workloads(txs: usize) -> Vec<(&'static str, Params)> {
    let base = Params {
        nodes: 10_000,
        txs,
        slots_per_tx: 2,
        p_new: 0.0,
        p_rev: 0.5,
        p_bad: 0.02,
        p_local: 0.3,
        seed: 0x5eed_0005,
    };
    vec![
        ("settled", base),
        ("mixed", Params { p_new: 0.2, ..base }),
        ("churn", Params { p_new: 0.5, ..base }),
        (
            "lazy",
            Params {
                p_new: 1.0,
                p_rev: 0.0,
                ..base
            },
        ),
    ]
}

/// The workload of that name, at [`TXS`] transactions.
pub fn workload(name: &str) -> Workload {
    let (_, p) = workloads(TXS)
        .into_iter()
        .find(|(n, _)| *n == name)
        .expect("no such workload");
    generate(&p)
}

/// A generated workload: the graph before the first transaction, the
/// transactions, and whether each is refused, by the generator's own
/// walk over the final graph.
#[derive(Clone)]
pub struct Workload {
    pub graph: Graph,
    pub txs: Vec<Tx>,
    pub refused: Vec<bool>,
}

impl Workload {
    pub fn moves(&self) -> usize {
        self.txs.iter().map(|t| t.moves.len()).sum()
    }

    /// Nodes built during the transactions.
    pub fn built(&self) -> usize {
        self.txs
            .iter()
            .flat_map(|t| &t.builds)
            .map(Build::len)
            .sum()
    }

    /// The subgraphs built during the transactions, in order, to measure
    /// building apart from moving.
    pub fn builds(&self) -> Vec<Build> {
        self.txs
            .iter()
            .flat_map(|t| t.builds.iter().cloned())
            .collect()
    }
}

#[derive(Clone, Copy)]
struct Screen {
    view: Id,
    events: Id,
}

struct Slot {
    view: Id,
    events: Id,
    screens: Vec<Screen>,
    current: usize,
    previous: Option<usize>,
    /// What a screen built for this slot takes as props.
    props: Vec<Id>,
}

struct Local {
    switch: Id,
    choices: Vec<Id>,
    current: usize,
}

struct Gen {
    g: Graph,
    rng: Rng,
    inputs: Vec<Id>,
    state: Vec<Id>,
    slots: Vec<Slot>,
    locals: Vec<Local>,
}

impl Gen {
    fn new_sub(&mut self) -> u32 {
        self.g.subs += 1;
        self.g.subs - 1
    }

    /// One to `k` distinct picks from `from`.
    fn some(&mut self, from: &[Id], k: usize) -> Vec<Id> {
        let mut out: Vec<Id> = Vec::new();
        for _ in 0..self.rng.range(1, k) {
            let d = self.rng.pick(from);
            if !out.contains(&d) {
                out.push(d);
            }
        }
        out
    }

    /// Combines `inputs` into one node, `fan` at a time, the way a view
    /// combines its children's views or an event merge its children's.
    fn fold(&mut self, sub: u32, mut inputs: Vec<Id>, fan: usize) -> Id {
        while inputs.len() > 1 {
            let rest = inputs.split_off(fan.min(inputs.len()));
            let n = self.g.add(sub, &inputs, false);
            inputs = rest;
            inputs.insert(0, n);
        }
        inputs[0]
    }

    /// Builds a component with `budget` components in its subtree, itself
    /// included, over `props`.
    fn component(&mut self, props: &[Id], budget: usize) -> Screen {
        let sub = self.new_sub();
        let mut reads = props.to_vec();
        for _ in 0..self.rng.range(2, 4) {
            reads.push(self.rng.pick(&self.state));
        }
        // Lifts over the props, the state, and each other, as a chain
        // that also reads one more of them at each step, so the view at its
        // end reads most of what the component reads.
        let mut lifts: Vec<Id> = Vec::new();
        for _ in 0..self.rng.range(4, 8) {
            let mut from = reads.clone();
            from.extend_from_slice(&lifts);
            let mut deps = self.some(&from, 2);
            if let Some(&prev) = lifts.last()
                && !deps.contains(&prev)
            {
                deps.push(prev);
            }
            lifts.push(self.g.add(sub, &deps, false));
        }
        // Events derived from inputs, one of them held and lifted.
        let mut events = Vec::new();
        for _ in 0..self.rng.range(2, 4) {
            let mut from = vec![self.rng.pick(&self.inputs)];
            if !events.is_empty() && self.rng.chance(0.5) {
                from.push(self.rng.pick(&events));
            }
            events.push(self.g.add(sub, &from, false));
        }
        let hold = self.g.add(sub, &[*events.last().unwrap()], false);
        let last = *lifts.last().unwrap();
        lifts.push(self.g.add(sub, &[hold, last], false));

        let mut views = vec![*lifts.last().unwrap()];
        let mut outs = vec![*events.last().unwrap()];
        if self.rng.chance(0.3) {
            let choices = self.some(&lifts, 3);
            let switch = self.g.add(sub, &[choices[0]], true);
            self.locals.push(Local {
                switch,
                choices,
                current: 0,
            });
            views.push(switch);
        }

        // Children: the rest of the budget, split between one or two static
        // children and at most one navigation slot of two or three screens.
        let left = budget - 1;
        let child_props: Vec<Id> = self.some(&lifts, 3);
        if left > 0 {
            let slot = left >= 2 && self.rng.chance(0.7);
            let screens = if slot {
                self.rng.range(2, 3.min(left))
            } else {
                0
            };
            let statics = if left > screens {
                self.rng.range(1, 2).min(left - screens)
            } else {
                0
            };
            let parts = screens + statics;
            let mut shares = vec![left / parts; parts];
            for s in shares.iter_mut().take(left % parts) {
                *s += 1;
            }
            for &share in &shares[screens..] {
                let c = self.component(&child_props, share);
                views.push(c.view);
                outs.push(c.events);
            }
            if slot {
                let built: Vec<Screen> = shares[..screens]
                    .iter()
                    .map(|&share| self.component(&child_props, share))
                    .collect();
                let view = self.g.add(sub, &[built[0].view], true);
                let events = self.g.add(sub, &[built[0].events], true);
                views.push(view);
                outs.push(events);
                self.slots.push(Slot {
                    view,
                    events,
                    screens: built,
                    current: 0,
                    previous: None,
                    props: child_props,
                });
            }
        }
        // The view combines two at a time, a chain a few nodes long; the
        // events merge three at a time.
        let view = self.fold(sub, views, 2);
        let events = self.fold(sub, outs, 3);
        Screen { view, events }
    }

    /// F46's gadget, in a subgraph of its own: A switches over `x` and `y`,
    /// B over `p` and `c`, `p = A + 10` and `y = B + 100`. Returns the moves
    /// that reverse it: B from `p` to `c` and A from `x` to `y`.
    fn f46(&mut self) -> [Move; 2] {
        let sub = self.new_sub();
        let x = self.g.add(sub, &[self.inputs[0]], false);
        let c = self.g.add(sub, &[self.inputs[1]], false);
        let a = self.g.add(sub, &[x], true);
        let p = self.g.add(sub, &[a], false);
        let b = self.g.add(sub, &[p], true);
        let y = self.g.add(sub, &[b], false);
        [
            Move {
                switch: b,
                from: p,
                to: c,
                kind: Kind::F46,
            },
            Move {
                switch: a,
                from: x,
                to: y,
                kind: Kind::F46,
            },
        ]
    }
}

/// Every move unlinked, then linked, then each checked by a walk over the
/// final graph; all undone if any closes a cycle. The generator's verdict
/// and the baseline's are this same code.
fn apply_and_walk(g: &mut Graph, moves: &[Move], walk: &mut Walk, visited: &mut [u64]) -> bool {
    for m in moves {
        g.unlink(m.from, m.switch);
    }
    for m in moves {
        g.link(m.to, m.switch);
    }
    let mut ok = true;
    for m in moves {
        if g.upstream(m.to, m.switch, walk, &mut visited[m.kind as usize]) {
            ok = false;
            break;
        }
    }
    if !ok {
        for m in moves {
            g.unlink(m.to, m.switch);
        }
        for m in moves {
            g.link(m.from, m.switch);
        }
    }
    ok
}

/// Generates a workload. Deterministic in `p`.
pub fn generate(p: &Params) -> Workload {
    let mut b = Gen {
        g: Graph::default(),
        rng: Rng(p.seed),
        inputs: Vec::new(),
        state: Vec::new(),
        slots: Vec::new(),
        locals: Vec::new(),
    };
    let root = b.new_sub();
    for _ in 0..64 {
        b.inputs.push(b.g.add(root, &[], false));
    }
    // The root event stream is a forward token, closed over the root
    // component's events once they exist. Holds on it, and lifts of them.
    let forward = b.g.add(root, &[], false);
    for _ in 0..60 {
        b.state.push(b.g.add(root, &[forward], false));
    }
    for _ in 0..140 {
        let deps = b.some(&b.state.clone(), 2);
        b.state.push(b.g.add(root, &deps, false));
    }
    let f46 = b.f46();
    // About 13 nodes a component, from a trial run of this generator.
    let budget = (p.nodes - b.g.len()) / 13;
    let props = b.some(&b.state.clone(), 3);
    let top = b.component(&props, budget);
    b.g.link(top.events, forward);
    let sink = b.g.add(root, &[top.view], false);
    let graph = b.g.clone();

    let mut walk = Walk::default();
    let mut scratch = [0; 5];
    let mut txs = Vec::with_capacity(p.txs);
    let mut refused = Vec::with_capacity(p.txs);
    for t in 0..p.txs {
        // F46's reversal halfway through, and back three quarters in.
        if t == p.txs / 2 || t == 3 * p.txs / 4 {
            let moves: Vec<Move> = if t == p.txs / 2 {
                f46.to_vec()
            } else {
                f46.iter()
                    .rev()
                    .map(|m| Move {
                        from: m.to,
                        to: m.from,
                        ..*m
                    })
                    .collect()
            };
            let ok = apply_and_walk(&mut b.g, &moves, &mut walk, &mut scratch);
            txs.push(Tx {
                builds: Vec::new(),
                moves,
            });
            refused.push(!ok);
            continue;
        }
        let mut builds = Vec::new();
        let mut moves = Vec::new();
        // Navigation happens in what is on screen: slots whose view switch
        // is upstream of the root view.
        let mut unused = 0;
        b.g.upstream(sink, Id::MAX, &mut walk, &mut unused);
        let visible: Vec<usize> = (0..b.slots.len())
            .filter(|&s| walk.seen(b.slots[s].view))
            .collect();
        let mut chosen: Vec<usize> = Vec::new();
        while chosen.len() < p.slots_per_tx.min(visible.len()) {
            let s = visible[b.rng.below(visible.len())];
            if !chosen.contains(&s) {
                chosen.push(s);
            }
        }
        let mut undo = Vec::new();
        for &s in &chosen {
            let (current, previous) = (b.slots[s].current, b.slots[s].previous);
            let target = if b.rng.chance(p.p_new) {
                let first = b.g.len();
                let props = b.slots[s].props.clone();
                let screen = b.component(&props, 1);
                builds.push(Build {
                    sub: b.g.sub[first],
                    nodes: (first..b.g.len())
                        .map(|n| (b.g.deps[n].clone(), b.g.switch[n]))
                        .collect(),
                });
                b.slots[s].screens.push(screen);
                b.slots[s].screens.len() - 1
            } else {
                match previous {
                    Some(prev) if b.rng.chance(p.p_rev) => prev,
                    _ => {
                        let n = b.slots[s].screens.len();
                        (current + 1 + b.rng.below(n - 1)) % n
                    }
                }
            };
            let slot = &mut b.slots[s];
            let (from, to) = (slot.screens[current], slot.screens[target]);
            moves.push(Move {
                switch: slot.view,
                from: from.view,
                to: to.view,
                kind: Kind::View,
            });
            moves.push(Move {
                switch: slot.events,
                from: from.events,
                to: to.events,
                kind: Kind::Event,
            });
            undo.push((s, current, previous));
            slot.previous = Some(current);
            slot.current = target;
        }
        let mut local_undo = None;
        if b.rng.chance(p.p_local) {
            let l = b.rng.below(b.locals.len());
            let local = &mut b.locals[l];
            if local.choices.len() > 1 {
                let n = local.choices.len();
                let next = (local.current + 1 + b.rng.below(n - 1)) % n;
                moves.push(Move {
                    switch: local.switch,
                    from: local.choices[local.current],
                    to: local.choices[next],
                    kind: Kind::Local,
                });
                local_undo = Some((l, local.current));
                local.current = next;
            }
        }
        if b.rng.chance(p.p_bad) {
            // Retarget the first view move at something downstream of its
            // switch in the graph before this transaction. The other moves
            // may break that path, and then the transaction stands.
            let m = moves[0];
            let mut down = Vec::new();
            walk.begin(b.g.len());
            walk.stack.push(m.switch);
            walk.mark(m.switch);
            while let Some(n) = walk.stack.pop() {
                down.push(n);
                for &d in &b.g.dependents[n as usize] {
                    if !walk.seen(d) {
                        walk.mark(d);
                        walk.stack.push(d);
                    }
                }
            }
            let to = down[1 + b.rng.below(down.len() - 1)];
            moves[0] = Move {
                to,
                kind: Kind::Bad,
                ..m
            };
        }
        let ok = apply_and_walk(&mut b.g, &moves, &mut walk, &mut scratch);
        if !ok {
            for &(s, current, previous) in undo.iter().rev() {
                b.slots[s].current = current;
                b.slots[s].previous = previous;
            }
            if let Some((l, current)) = local_undo {
                b.locals[l].current = current;
            }
        } else if moves[0].kind == Kind::Bad {
            // The bad target stands as the slot's view; its screen index
            // no longer describes it, so later moves start from it.
            let s = chosen[0];
            let slot = &mut b.slots[s];
            let events = slot.screens[slot.current].events;
            slot.screens.push(Screen {
                view: moves[0].to,
                events,
            });
            slot.current = slot.screens.len() - 1;
        }
        txs.push(Tx { builds, moves });
        refused.push(!ok);
    }
    Workload {
        graph,
        txs,
        refused,
    }
}

/// What a checker did, per kind of move: how many moves, how many nodes
/// it visited (walked, searched, or expanded), and for [`Pk`] how many
/// links invalidated the order.
#[derive(Clone, Default, Debug)]
pub struct Counts {
    pub moves: [u64; 5],
    pub visited: [u64; 5],
    pub invalid: [u64; 5],
    /// [`Summaries`] only: subgraphs rebuilt at commit, and their nodes.
    pub rebuilds: u64,
    pub rebuilt: u64,
    pub refused: u64,
}

/// A same-instant cycle check at commit, with whatever it maintains.
pub trait Checker: Clone {
    fn new(g: &Graph) -> Self;
    /// The nodes from `first` on were just built, as one new subgraph.
    fn built(&mut self, g: &Graph, first: Id);
    /// Moves every switch and checks the final graph. Returns whether the
    /// transaction is accepted; if not, the moves are undone.
    fn commit(&mut self, g: &mut Graph, moves: &[Move]) -> bool;
    fn counts(&self) -> &Counts;
}

/// The spike's check.
#[derive(Clone, Default)]
pub struct Baseline {
    walk: Walk,
    counts: Counts,
}

impl Checker for Baseline {
    fn new(_: &Graph) -> Self {
        Self::default()
    }

    fn built(&mut self, _: &Graph, _: Id) {}

    fn commit(&mut self, g: &mut Graph, moves: &[Move]) -> bool {
        for m in moves {
            self.counts.moves[m.kind as usize] += 1;
        }
        let ok = apply_and_walk(g, moves, &mut self.walk, &mut self.counts.visited);
        self.counts.refused += !ok as u64;
        ok
    }

    fn counts(&self) -> &Counts {
        &self.counts
    }
}

/// Kahn's algorithm over `nodes`, following only edges between nodes
/// `inside` says belong. Returns them in a topological order, or `None` on
/// a cycle. `indeg` is scratch indexed by node.
fn kahn(
    g: &Graph,
    nodes: &[Id],
    inside: impl Fn(Id) -> bool,
    indeg: &mut [u32],
) -> Option<Vec<Id>> {
    for &n in nodes {
        indeg[n as usize] = g.deps[n as usize].iter().filter(|&&d| inside(d)).count() as u32;
    }
    let mut order: Vec<Id> = nodes
        .iter()
        .copied()
        .filter(|&n| indeg[n as usize] == 0)
        .collect();
    let mut k = 0;
    while k < order.len() {
        let n = order[k];
        k += 1;
        for &d in &g.dependents[n as usize] {
            if inside(d) {
                indeg[d as usize] -= 1;
                if indeg[d as usize] == 0 {
                    order.push(d);
                }
            }
        }
    }
    (order.len() == nodes.len()).then_some(order)
}

/// Pearce and Kelly's dynamic topological order: a `u32` position per
/// node, in an array, not an order-maintenance list.
#[derive(Clone, Default)]
pub struct Pk {
    ord: Vec<u32>,
    /// The next position to append at. Nothing is collected here, so no
    /// position is freed; a reorder reuses the positions it found.
    next: u32,
    walk: Walk,
    forward: Vec<Id>,
    backward: Vec<Id>,
    pool: Vec<u32>,
    counts: Counts,
}

impl Pk {
    /// Links `x → y` unless it closes a cycle, keeping the order.
    fn insert(&mut self, g: &mut Graph, x: Id, y: Id, kind: Kind) -> bool {
        let (lb, ub) = (self.ord[y as usize], self.ord[x as usize]);
        if lb > ub {
            g.link(x, y);
            return true;
        }
        self.counts.invalid[kind as usize] += 1;
        // Forward from y over dependents, only before x's position; the
        // node at x's position is x, and reaching it is a cycle.
        let walk = &mut self.walk;
        walk.begin(g.len());
        self.forward.clear();
        walk.stack.push(y);
        walk.mark(y);
        while let Some(n) = walk.stack.pop() {
            self.forward.push(n);
            for &w in &g.dependents[n as usize] {
                let o = self.ord[w as usize];
                if o == ub {
                    self.counts.visited[kind as usize] += self.forward.len() as u64;
                    return false;
                }
                if o < ub && !walk.seen(w) {
                    walk.mark(w);
                    walk.stack.push(w);
                }
            }
        }
        // Backward from x over dependencies, only after y's position.
        self.backward.clear();
        walk.stack.push(x);
        walk.mark(x);
        while let Some(n) = walk.stack.pop() {
            self.backward.push(n);
            for &w in &g.deps[n as usize] {
                if self.ord[w as usize] > lb && !walk.seen(w) {
                    walk.mark(w);
                    walk.stack.push(w);
                }
            }
        }
        self.counts.visited[kind as usize] += (self.forward.len() + self.backward.len()) as u64;
        // Everything that reaches x goes first, in its old relative order,
        // then everything y reaches, into the positions they held.
        let ord = &mut self.ord;
        self.backward.sort_unstable_by_key(|&n| ord[n as usize]);
        self.forward.sort_unstable_by_key(|&n| ord[n as usize]);
        self.pool.clear();
        self.pool.extend(
            self.backward
                .iter()
                .chain(&self.forward)
                .map(|&n| ord[n as usize]),
        );
        self.pool.sort_unstable();
        for (&n, &o) in self.backward.iter().chain(&self.forward).zip(&self.pool) {
            ord[n as usize] = o;
        }
        g.link(x, y);
        true
    }
}

impl Checker for Pk {
    fn new(g: &Graph) -> Self {
        let all: Vec<Id> = (0..g.len() as Id).collect();
        let mut indeg = vec![0; g.len()];
        let order = kahn(g, &all, |_| true, &mut indeg).expect("the build is acyclic");
        let mut ord = vec![0; g.len()];
        for (k, &n) in order.iter().enumerate() {
            ord[n as usize] = k as u32;
        }
        Pk {
            ord,
            next: g.len() as u32,
            ..Pk::default()
        }
    }

    /// A node built depends only on older ones, so it goes at the end.
    fn built(&mut self, g: &Graph, first: Id) {
        for _ in first as usize..g.len() {
            self.ord.push(self.next);
            self.next += 1;
        }
    }

    fn commit(&mut self, g: &mut Graph, moves: &[Move]) -> bool {
        for m in moves {
            self.counts.moves[m.kind as usize] += 1;
        }
        // Deletions first: they never invalidate the order, and after them
        // each graph on the way is a subgraph of the final one (F46).
        for m in moves {
            g.unlink(m.from, m.switch);
        }
        let mut linked = 0;
        while linked < moves.len() {
            let m = moves[linked];
            if !self.insert(g, m.to, m.switch, m.kind) {
                break;
            }
            linked += 1;
        }
        if linked == moves.len() {
            return true;
        }
        // Refused: undo what was linked, and relink the old inners through
        // the order, since the reorders may have put one after its switch.
        // The old graph was acyclic, so this cannot fail.
        self.counts.refused += 1;
        for m in &moves[..linked] {
            g.unlink(m.to, m.switch);
        }
        for m in moves {
            let relinked = self.insert(g, m.from, m.switch, m.kind);
            debug_assert!(relinked);
        }
        false
    }

    fn counts(&self) -> &Counts {
        &self.counts
    }
}

/// Per-subgraph reachability summaries.
#[derive(Clone, Default)]
pub struct Summaries {
    /// Each subgraph's nodes, and its sources in bit order.
    nodes: Vec<Vec<Id>>,
    sources: Vec<Vec<Id>>,
    /// For each node, which of its subgraph's sources reach it inside the
    /// subgraph, itself included if it is one.
    reach: Vec<u64>,
    bit: Vec<u8>,
    stale: Vec<u32>,
    indeg: Vec<u32>,
    outs: Walk,
    srcs: Walk,
    counts: Counts,
}

impl Summaries {
    /// Recomputes `sub`'s summary; `false` if the subgraph has a cycle.
    /// `in_order` says its nodes are already topologically ordered, as a
    /// subgraph just built is.
    fn rebuild(&mut self, g: &Graph, sub: u32, in_order: bool) -> bool {
        let nodes = std::mem::take(&mut self.nodes[sub as usize]);
        let inside = |d: Id| g.sub[d as usize] == sub;
        let source = |n: Id| g.switch[n as usize] || g.deps[n as usize].iter().any(|&d| !inside(d));
        let sources: Vec<Id> = nodes.iter().copied().filter(|&n| source(n)).collect();
        assert!(sources.len() <= 64, "a subgraph with more than 64 sources");
        for (k, &s) in sources.iter().enumerate() {
            self.bit[s as usize] = k as u8;
        }
        let order = if in_order {
            Some(nodes.clone())
        } else {
            kahn(g, &nodes, inside, &mut self.indeg)
        };
        let ok = order.is_some();
        for n in order.unwrap_or_default() {
            let mut r = if source(n) {
                1u64 << self.bit[n as usize]
            } else {
                0
            };
            for &d in &g.deps[n as usize] {
                if inside(d) {
                    r |= self.reach[d as usize];
                }
            }
            self.reach[n as usize] = r;
        }
        self.nodes[sub as usize] = nodes;
        self.sources[sub as usize] = sources;
        ok
    }

    /// Whether `s` is upstream of `x`, walking summaries: from each node
    /// reached, the sources that reach it in its subgraph, and from each
    /// source, its dependencies outside the subgraph.
    fn upstream(&mut self, g: &Graph, x: Id, s: Id, kind: Kind) -> bool {
        self.outs.begin(g.len());
        self.srcs.begin(g.len());
        self.outs.stack.push(x);
        self.outs.mark(x);
        let mut count = 0;
        let mut found = false;
        'walk: while let Some(n) = self.outs.stack.pop() {
            count += 1;
            let sub = g.sub[n as usize];
            let mut bits = self.reach[n as usize];
            while bits != 0 {
                let q = self.sources[sub as usize][bits.trailing_zeros() as usize];
                bits &= bits - 1;
                if q == s {
                    found = true;
                    break 'walk;
                }
                if self.srcs.seen(q) {
                    continue;
                }
                self.srcs.mark(q);
                count += 1;
                for &d in &g.deps[q as usize] {
                    if g.sub[d as usize] != sub && !self.outs.seen(d) {
                        self.outs.mark(d);
                        self.outs.stack.push(d);
                    }
                }
            }
        }
        self.counts.visited[kind as usize] += count;
        found
    }

    /// Applies the moves, backwards if `undo`, marks stale the subgraph of
    /// any switch whose old or new inner is inside it, and rebuilds those.
    fn relink(&mut self, g: &mut Graph, moves: &[Move], undo: bool) -> bool {
        self.stale.clear();
        for m in moves {
            let (from, to) = if undo { (m.to, m.from) } else { (m.from, m.to) };
            g.unlink(from, m.switch);
            let sub = g.sub[m.switch as usize];
            let local = g.sub[from as usize] == sub || g.sub[to as usize] == sub;
            if local && !self.stale.contains(&sub) {
                self.stale.push(sub);
            }
        }
        for m in moves {
            g.link(if undo { m.from } else { m.to }, m.switch);
        }
        let mut ok = true;
        for k in 0..self.stale.len() {
            let sub = self.stale[k];
            self.counts.rebuilds += 1;
            self.counts.rebuilt += self.nodes[sub as usize].len() as u64;
            ok &= self.rebuild(g, sub, false);
        }
        ok
    }
}

impl Checker for Summaries {
    fn new(g: &Graph) -> Self {
        let mut me = Summaries {
            nodes: vec![Vec::new(); g.subs as usize],
            sources: vec![Vec::new(); g.subs as usize],
            reach: vec![0; g.len()],
            bit: vec![0; g.len()],
            indeg: vec![0; g.len()],
            ..Summaries::default()
        };
        for n in 0..g.len() {
            me.nodes[g.sub[n] as usize].push(n as Id);
        }
        for sub in 0..g.subs {
            let ok = me.rebuild(g, sub, false);
            assert!(ok, "the build is acyclic");
        }
        me
    }

    fn built(&mut self, g: &Graph, first: Id) {
        let sub = g.sub[first as usize] as usize;
        self.reach.resize(g.len(), 0);
        self.bit.resize(g.len(), 0);
        self.indeg.resize(g.len(), 0);
        if self.nodes.len() <= sub {
            self.nodes.resize(sub + 1, Vec::new());
            self.sources.resize(sub + 1, Vec::new());
        }
        self.nodes[sub] = (first..g.len() as Id).collect();
        self.rebuild(g, sub as u32, true);
    }

    fn commit(&mut self, g: &mut Graph, moves: &[Move]) -> bool {
        for m in moves {
            self.counts.moves[m.kind as usize] += 1;
        }
        let mut ok = self.relink(g, moves, false);
        if ok {
            for m in moves {
                if self.upstream(g, m.to, m.switch, m.kind) {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            self.counts.refused += 1;
            let restored = self.relink(g, moves, true);
            debug_assert!(restored);
        }
        ok
    }

    fn counts(&self) -> &Counts {
        &self.counts
    }
}

/// A graph and a checker, ready to run a workload's transactions.
#[derive(Clone)]
pub struct Run<C> {
    pub graph: Graph,
    pub checker: C,
}

impl<C: Checker> Run<C> {
    pub fn new(w: &Workload) -> Self {
        Run {
            graph: w.graph.clone(),
            checker: C::new(&w.graph),
        }
    }

    /// Builds one subgraph, as a transaction does during its instant.
    pub fn build(&mut self, b: &Build) {
        let first = self.graph.len() as Id;
        for (deps, switch) in &b.nodes {
            self.graph.add(b.sub, deps, *switch);
        }
        self.checker.built(&self.graph, first);
    }

    /// One transaction: its builds, then its commit.
    pub fn tx(&mut self, t: &Tx) -> bool {
        for b in &t.builds {
            self.build(b);
        }
        self.checker.commit(&mut self.graph, &t.moves)
    }

    /// Every transaction; returns how many were refused.
    pub fn all(&mut self, txs: &[Tx]) -> usize {
        txs.iter().filter(|t| !self.tx(t)).count()
    }
}

/// The median and largest upstream of a view move's new inner, over the
/// graph before the first transaction, for inners that existed then.
pub fn view_upstream(w: &Workload) -> (u64, u64) {
    let g = &w.graph;
    let mut walk = Walk::default();
    let mut sizes = Vec::new();
    for t in &w.txs {
        for m in t.moves.iter().filter(|m| m.kind == Kind::View) {
            if (m.to as usize) < g.len() {
                let mut v = 0;
                g.upstream(m.to, Id::MAX, &mut walk, &mut v);
                sizes.push(v);
            }
        }
    }
    sizes.sort_unstable();
    if sizes.is_empty() {
        (0, 0)
    } else {
        (sizes[sizes.len() / 2], sizes[sizes.len() - 1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F46 alone: x, c; A over x; p = A + 10; B over p; y = B + 100.
    /// Moving A to y and B to c together is legal, though A's move alone
    /// would read as a cycle.
    fn f46() -> (Graph, Vec<Move>) {
        let mut g = Graph::default();
        let x = g.add(0, &[], false);
        let c = g.add(0, &[], false);
        let a = g.add(1, &[x], true);
        let p = g.add(1, &[a], false);
        let b = g.add(1, &[p], true);
        let y = g.add(1, &[b], false);
        let moves = vec![
            Move {
                switch: a,
                from: x,
                to: y,
                kind: Kind::F46,
            },
            Move {
                switch: b,
                from: p,
                to: c,
                kind: Kind::F46,
            },
        ];
        (g, moves)
    }

    fn f46_accepted<C: Checker>() {
        let (mut g, moves) = f46();
        let mut checker = C::new(&g);
        assert!(checker.commit(&mut g, &moves), "F46's reversal refused");
        // Now B back to p alone closes y → A → p → B → y: refused.
        let back = [Move {
            from: moves[1].to,
            to: moves[1].from,
            ..moves[1]
        }];
        assert!(!checker.commit(&mut g, &back), "a cycle accepted");
        // And both back together is legal again.
        let both: Vec<Move> = moves
            .iter()
            .map(|m| Move {
                from: m.to,
                to: m.from,
                ..*m
            })
            .collect();
        assert!(checker.commit(&mut g, &both), "F46 undone refused");
    }

    #[test]
    fn f46_is_accepted_by_every_checker() {
        f46_accepted::<Baseline>();
        f46_accepted::<Pk>();
        f46_accepted::<Summaries>();
    }

    fn agrees<C: Checker>(w: &Workload) -> Counts {
        let mut run = Run::<C>::new(w);
        let refused: Vec<bool> = w.txs.iter().map(|t| !run.tx(t)).collect();
        assert_eq!(refused, w.refused, "verdicts differ from the walk's");
        run.checker.counts().clone()
    }

    /// Every checker refuses exactly the transactions the walk refuses, on
    /// every workload, also with many more cyclic moves.
    #[test]
    fn every_checker_refuses_what_the_walk_refuses() {
        for (_, mut p) in workloads(120) {
            for bad in [p.p_bad, 0.3] {
                p.p_bad = bad;
                let w = generate(&p);
                assert!(bad < 0.3 || w.refused.iter().any(|&r| r));
                agrees::<Baseline>(&w);
                agrees::<Pk>(&w);
                agrees::<Summaries>(&w);
            }
        }
    }

    /// The counts the note quotes: per workload and kind of move, how many
    /// nodes each checker visits, and how often a link invalidates the
    /// order. Run with `--nocapture`.
    #[test]
    fn counts() {
        println!("{TXS} transactions per workload, 2 slots each");
        for (name, p) in workloads(TXS) {
            let w = generate(&p);
            let base = agrees::<Baseline>(&w);
            let pk = agrees::<Pk>(&w);
            let sum = agrees::<Summaries>(&w);
            let (median, max) = view_upstream(&w);
            println!();
            println!(
                "{name} (p_new {}, p_rev {}): {} nodes in {} subgraphs, {} built in {} subgraphs \
                 during the transactions, {} moves, {} transactions refused",
                p.p_new,
                p.p_rev,
                w.graph.len(),
                w.graph.subs,
                w.built(),
                w.builds().len(),
                w.moves(),
                base.refused,
            );
            if max > 0 {
                println!(
                    "  upstream of a view inner that existed before the first transaction: \
                     median {median}, max {max} nodes"
                );
            }
            println!(
                "  {:<6} {:>6} {:>11} {:>11} {:>11} {:>11}",
                "kind", "moves", "walk", "pk", "pk invalid", "summaries"
            );
            let row = |label: &str, n: u64, b: u64, k: u64, inv: u64, s: u64| {
                let n = n as f64;
                println!(
                    "  {label:<6} {:>6} {:>11.1} {:>11.1} {:>10.1}% {:>11.1}",
                    n,
                    b as f64 / n,
                    k as f64 / n,
                    100.0 * inv as f64 / n,
                    s as f64 / n,
                );
            };
            let mut total = [0; 5];
            for k in KINDS {
                let i = k as usize;
                if base.moves[i] == 0 {
                    continue;
                }
                let r = [
                    base.moves[i],
                    base.visited[i],
                    pk.visited[i],
                    pk.invalid[i],
                    sum.visited[i],
                ];
                row(
                    &format!("{k:?}").to_lowercase(),
                    r[0],
                    r[1],
                    r[2],
                    r[3],
                    r[4],
                );
                for (t, x) in total.iter_mut().zip(r) {
                    *t += x;
                }
            }
            row("all", total[0], total[1], total[2], total[3], total[4]);
            println!(
                "  summaries rebuilt at commit: {} subgraphs, {} nodes, {:.1} a move",
                sum.rebuilds,
                sum.rebuilt,
                sum.rebuilt as f64 / total[0] as f64,
            );
        }
        println!();
        println!("nodes visited per move: walked (walk), searched forward and back (pk),");
        println!("or outputs and sources expanded (summaries); pk invalid is the share of");
        println!("moves whose new link was against the order. A refused transaction's");
        println!("relinks are counted under their moves' kinds.");
    }
}
