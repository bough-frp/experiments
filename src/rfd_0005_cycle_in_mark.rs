//! Can the depth-first mark's grey state replace the per-move upstream walk
//! as RFD 5's same-instant cycle check?
//!
//! RFD 5 marks the affected region with a depth-first walk over dependents
//! from the fired inputs and evaluates its reverse post-order. Separately,
//! commit moves every switch and then walks upstream from each new inner
//! looking for the switch (F46, F50), and every read through a
//! `switch_cell`'s selection carries Brent's cycle detection (F19, F56,
//! F57). Sycamore's scheduler runs the same mark and panics when the walk
//! meets a node still on its stack, a grey node.
//!
//! This module is a model of that engine in the few hundred lines the
//! question needs: the dependency relation, the two marks (with and without
//! a grey state), the upstream walk, reads that chase switch selections
//! with Brent's guard, the pull that runs nodes built during an instant,
//! and relink in two passes. Values are left out except the tokens cells
//! hold, since a switch's selection is a token and nothing else here needs
//! a value. The binary runs the cases on it; the instruction-count bench
//! counts the walk against the two marks over F50's shape.

/// A node's index in the graph.
pub type Id = u32;

/// No node: an unlinked switch, a cell holding a number, no pending value.
pub const NONE: Id = u32::MAX;

/// How deep a read may recurse before the model calls it a stack overflow,
/// which in the engine aborts the process. Well under what the model's own
/// stack can take.
pub const READ_DEPTH_CAP: u32 = 10_000;

/// What a node is, as far as ordering and reads care.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// An input: where a mark starts.
    Input,
    /// Any stream computed from its dependencies: map, merge, filter.
    Stream,
    /// A stream that reads a cell's value before the instant when it
    /// fires. The read is not a dependency.
    Snapshot,
    /// A cell's steps: a stream that depends on the cell and reads its
    /// value after the instant.
    Steps,
    /// A constant cell.
    Const,
    /// A hold: a cell that depends on its stream and stores its value.
    Hold,
    /// A read-through cell (`map_cell`): no storage, every read reads its
    /// dependencies.
    ReadThrough,
    /// A loop's forward token: depends on its definition, and a read of it
    /// reads the definition.
    Loop,
    /// `switch_cell`: depends on its outer and on its linked inner; a read
    /// chases the outer's selection, not the link.
    SwitchCell,
    /// `switch_stream`: depends on its linked inner only. Its outer is a
    /// watcher, not a dependency, so a loop through its selection is legal.
    SwitchStream,
}

/// Brent's cycle detection over the `switch_cell`s a read passes, as the
/// spike's `Passed`: three integers down the call stack.
#[derive(Clone, Copy)]
pub struct Passed {
    saved: Id,
    since: u32,
    power: u32,
}

impl Passed {
    pub const NOTHING: Passed = Passed {
        saved: NONE,
        since: 0,
        power: 1,
    };

    /// The read passes switch `i`, or meets the one it saved: `None`.
    fn pass(self, i: Id) -> Option<Passed> {
        if i == self.saved {
            return None;
        }
        let since = self.since + 1;
        Some(if since == self.power {
            Passed {
                saved: i,
                since: 0,
                power: self.power.saturating_mul(2),
            }
        } else {
            Passed { since, ..self }
        })
    }
}

/// How a read failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadFail {
    /// Brent's guard met a switch it passed before.
    Guard(Id),
    /// No guard, and the read went past `READ_DEPTH_CAP`.
    Overflow,
}

/// The dependency graph, and the scratch the marks, the walk and the pull
/// reuse. Parallel vectors indexed by `Id`.
#[derive(Default)]
pub struct Graph {
    kind: Vec<Kind>,
    deps: Vec<Vec<Id>>,
    dependents: Vec<Vec<Id>>,
    /// A `switch_stream`'s outer lists it here instead of as a dependent.
    watchers: Vec<Vec<Id>>,
    /// A switch's outer.
    outer: Vec<Id>,
    /// A switch's linked inner.
    linked: Vec<Id>,
    /// The cell a snapshot or a steps reads.
    reads: Vec<Id>,
    /// The token a const or hold holds, or a read-through cell's function
    /// returns: `NONE` for a number.
    token: Vec<Id>,
    /// A hold's value after this instant, if its stream fired with one.
    pending: Vec<Id>,

    /// Mark stamps: `2 tx` is grey (on the walk's stack), `2 tx + 1` black.
    mark: Vec<u32>,
    tx: u32,
    /// The mark's post-order, and the switches it queued for relink.
    pub order: Vec<Id>,
    pub relinks: Vec<Id>,
    stack: Vec<(Id, u32)>,

    /// The upstream walk's visit epoch, and how many nodes the last walk
    /// visited.
    visit: Vec<u32>,
    epoch: u32,
    pub walked: u32,
    search: Vec<(Id, u32)>,

    /// The pull's stamps: `2 p` in progress, `2 p + 1` done.
    pull: Vec<u32>,
    pull_epoch: u32,
}

impl Graph {
    pub fn len(&self) -> Id {
        self.kind.len() as Id
    }

    pub fn is_empty(&self) -> bool {
        self.kind.is_empty()
    }

    pub fn kind(&self, n: Id) -> Kind {
        self.kind[n as usize]
    }

    pub fn deps(&self, n: Id) -> &[Id] {
        &self.deps[n as usize]
    }

    pub fn dependents(&self, n: Id) -> &[Id] {
        &self.dependents[n as usize]
    }

    pub fn watchers(&self, n: Id) -> &[Id] {
        &self.watchers[n as usize]
    }

    pub fn linked(&self, n: Id) -> Id {
        self.linked[n as usize]
    }

    fn node(&mut self, kind: Kind, deps: &[Id]) -> Id {
        let n = self.len();
        self.kind.push(kind);
        self.deps.push(Vec::new());
        self.dependents.push(Vec::new());
        self.watchers.push(Vec::new());
        self.outer.push(NONE);
        self.linked.push(NONE);
        self.reads.push(NONE);
        self.token.push(NONE);
        self.pending.push(NONE);
        self.mark.push(0);
        self.visit.push(0);
        self.pull.push(0);
        for &d in deps {
            self.edge(d, n);
        }
        n
    }

    fn edge(&mut self, from: Id, to: Id) {
        self.deps[to as usize].push(from);
        self.dependents[from as usize].push(to);
    }

    fn unedge(&mut self, from: Id, to: Id) {
        let d = &mut self.deps[to as usize];
        let at = d.iter().position(|&x| x == from).expect("no such edge");
        d.swap_remove(at);
        let d = &mut self.dependents[from as usize];
        let at = d.iter().position(|&x| x == to).expect("no such edge");
        d.swap_remove(at);
    }

    pub fn input(&mut self) -> Id {
        self.node(Kind::Input, &[])
    }

    pub fn stream(&mut self, deps: &[Id]) -> Id {
        self.node(Kind::Stream, deps)
    }

    pub fn snapshot(&mut self, stream: Id, cell: Id) -> Id {
        let n = self.node(Kind::Snapshot, &[stream]);
        self.reads[n as usize] = cell;
        n
    }

    pub fn steps(&mut self, cell: Id) -> Id {
        let n = self.node(Kind::Steps, &[cell]);
        self.reads[n as usize] = cell;
        n
    }

    /// A constant holding `token`, or a number if `NONE`.
    pub fn constant(&mut self, token: Id) -> Id {
        let n = self.node(Kind::Const, &[]);
        self.token[n as usize] = token;
        n
    }

    pub fn hold(&mut self, stream: Id, init: Id) -> Id {
        let n = self.node(Kind::Hold, &[stream]);
        self.token[n as usize] = init;
        n
    }

    /// A `map_cell` over `deps` whose function returns `yields`.
    pub fn map_cell(&mut self, deps: &[Id], yields: Id) -> Id {
        let n = self.node(Kind::ReadThrough, deps);
        self.token[n as usize] = yields;
        n
    }

    /// A cell or stream loop's forward token, not yet closed.
    pub fn forward(&mut self) -> Id {
        self.node(Kind::Loop, &[])
    }

    pub fn close(&mut self, forward: Id, definition: Id) {
        self.edge(definition, forward);
    }

    /// A `switch_cell`, unlinked until its first evaluation.
    pub fn switch_cell(&mut self, outer: Id) -> Id {
        let n = self.node(Kind::SwitchCell, &[outer]);
        self.outer[n as usize] = outer;
        n
    }

    /// A `switch_stream`, unlinked until its first evaluation.
    pub fn switch_stream(&mut self, outer: Id) -> Id {
        let n = self.node(Kind::SwitchStream, &[]);
        self.outer[n as usize] = outer;
        self.watchers[outer as usize].push(n);
        n
    }

    /// Points a switch's link at `inner`, dropping the old link's edge.
    pub fn link(&mut self, switch: Id, inner: Id) {
        let old = self.linked[switch as usize];
        if old != NONE {
            self.unedge(old, switch);
        }
        self.edge(inner, switch);
        self.linked[switch as usize] = inner;
    }

    /// Gives hold `h` a new value after this instant.
    pub fn set_pending(&mut self, h: Id, token: Id) {
        self.pending[h as usize] = token;
    }

    /// Commits every pending hold value.
    pub fn commit_holds(&mut self) {
        for n in 0..self.kind.len() {
            if self.pending[n] != NONE {
                self.token[n] = self.pending[n];
                self.pending[n] = NONE;
            }
        }
    }

    /// RFD 5's mark: an iterative depth-first walk over dependents from each
    /// fired input. Its post-order, read backwards, is the evaluation order.
    /// The mark queues every `switch_cell` it orders and every
    /// `switch_stream` watching a node it reaches, for relink at commit.
    ///
    /// With `GREY`, a node is grey while it is on the walk's stack and black
    /// once finished, and meeting a grey node is a cycle: `Err` names it.
    /// Without, a node is black as soon as it is reached, and a revisit is
    /// skipped, as in the spike's mark less its backstop. Both write one
    /// stamp per node on the way down; the grey mark writes a second on the
    /// way up and compares once more on a revisit.
    pub fn mark<const GREY: bool>(&mut self, starts: &[Id]) -> Result<(), Id> {
        self.tx += 1;
        let grey = 2 * self.tx;
        let black = grey + 1;
        let Graph {
            kind,
            dependents,
            watchers,
            mark,
            order,
            relinks,
            stack,
            ..
        } = self;
        order.clear();
        relinks.clear();
        stack.clear();
        for &start in starts {
            let (mut n, mut k) = (start, 0usize);
            loop {
                let ds = &dependents[n as usize];
                if k < ds.len() {
                    let d = ds[k];
                    k += 1;
                    let m = mark[d as usize];
                    if m < grey {
                        mark[d as usize] = if GREY { grey } else { black };
                        for &w in &watchers[d as usize] {
                            relinks.push(w);
                        }
                        stack.push((n, k as u32));
                        (n, k) = (d, 0);
                    } else if GREY && m == grey {
                        stack.clear();
                        return Err(d);
                    }
                } else {
                    // `n` is finished. The start, at the bottom, is not
                    // ordered.
                    let Some((parent, visited)) = stack.pop() else {
                        break;
                    };
                    if GREY {
                        mark[n as usize] = black;
                    }
                    if kind[n as usize] == Kind::SwitchCell {
                        relinks.push(n);
                    }
                    order.push(n);
                    (n, k) = (parent, visited as usize);
                }
            }
        }
        Ok(())
    }

    /// The per-move check: whether `target` is upstream of `from`, walking
    /// dependencies with a visit epoch, as the spike's `path`. Sets
    /// `walked` to the number of nodes visited.
    pub fn upstream_reaches(&mut self, from: Id, target: Id) -> bool {
        self.epoch += 1;
        let epoch = self.epoch;
        let Graph {
            deps,
            visit,
            search,
            walked,
            ..
        } = self;
        search.clear();
        search.push((from, 0));
        visit[from as usize] = epoch;
        *walked = 1;
        while let Some(top) = search.last_mut() {
            let (n, k) = *top;
            if n == target {
                search.clear();
                return true;
            }
            let next = &deps[n as usize];
            if (k as usize) < next.len() {
                top.1 += 1;
                let d = next[k as usize];
                if visit[d as usize] != epoch {
                    visit[d as usize] = epoch;
                    *walked += 1;
                    search.push((d, 0));
                }
            } else {
                search.pop();
            }
        }
        false
    }

    /// The token cell `n` holds, before the instant or, with `post`, after
    /// it. A read-through cell reads its dependencies, a loop its
    /// definition, and a `switch_cell` the cell its outer selects, passing
    /// the switch through Brent's guard if `guard` is on.
    pub fn read(
        &self,
        n: Id,
        post: bool,
        guard: bool,
        passed: Passed,
        depth: u32,
    ) -> Result<Id, ReadFail> {
        if depth > READ_DEPTH_CAP {
            return Err(ReadFail::Overflow);
        }
        let i = n as usize;
        match self.kind[i] {
            Kind::Const => Ok(self.token[i]),
            Kind::Hold => Ok(if post && self.pending[i] != NONE {
                self.pending[i]
            } else {
                self.token[i]
            }),
            Kind::ReadThrough => {
                for &d in &self.deps[i] {
                    self.read(d, post, guard, passed, depth + 1)?;
                }
                Ok(self.token[i])
            }
            Kind::Loop => self.read(self.deps[i][0], post, guard, passed, depth + 1),
            Kind::SwitchCell => {
                let passed = if guard {
                    passed.pass(n).ok_or(ReadFail::Guard(n))?
                } else {
                    passed
                };
                let selected = self.read(self.outer[i], post, guard, passed, depth + 1)?;
                self.read(selected, post, guard, passed, depth + 1)
            }
            _ => Ok(NONE),
        }
    }
}

/// Which cycle check an engine runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Design {
    /// RFD 5 as written: plain mark, and every first link and every move
    /// walked upstream, moves once all switches have moved.
    Walk,
    /// The spike's rejected first try: each move walked as it is made.
    WalkEachMove,
    /// The proposal: a grey mark, and no walk.
    Grey,
}

/// Where in an instant a cycle was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Building the graph, transaction zero: a construction panic.
    Build,
    Mark,
    Evaluate,
    /// Running a `construct` closure and pulling the nodes it built.
    Construct,
    Relink,
    Dispatch,
    /// A `Runtime::sample` between transactions, which RFD 5 does not
    /// count as escaping a transaction.
    Sample,
}

/// What found it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum By {
    Walk,
    GreyMark,
    /// A read's Brent guard.
    ReadGuard,
    /// The pull's in-progress stamp over nodes built during the instant.
    PullGuard,
    /// Nothing: a read went past the depth cap, a stack overflow.
    Overflow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Found {
    pub tx: u32,
    pub phase: Phase,
    pub by: By,
}

/// A `construct` closure: builds nodes and returns the cells it samples,
/// in order.
pub type Closure = Box<dyn FnOnce(&mut Graph) -> Vec<Id>>;

/// One transaction of a script.
#[derive(Default)]
pub struct Tx {
    pub fires: Vec<Id>,
    /// Holds whose stream fires with a new token.
    pub sets: Vec<(Id, Id)>,
    pub construct: Option<Closure>,
    /// Cells with a listener, read at dispatch if the mark reached them.
    pub listen: Vec<Id>,
}

/// What a script does next.
pub enum Item {
    Tx(Tx),
    Sample(Id),
}

/// The model engine: a graph, a design, and the first cycle found, after
/// which it is poisoned and runs nothing more.
pub struct Engine {
    pub g: Graph,
    pub design: Design,
    pub brent: bool,
    /// Relink in descending node order instead of ascending: the queue
    /// order depends on marking, and F46's one-move-at-a-time refusal
    /// shows in only one of them.
    pub reverse_relinks: bool,
    pub tx: u32,
    pub found: Option<Found>,
}

impl Engine {
    /// Runs the build closure as transaction zero: its samples, then the
    /// pull of every node it built.
    pub fn build(design: Design, brent: bool, f: impl FnOnce(&mut Graph) -> Vec<Id>) -> Engine {
        let mut e = Engine {
            g: Graph::default(),
            design,
            brent,
            reverse_relinks: false,
            tx: 0,
            found: None,
        };
        let samples = f(&mut e.g);
        e.found = e.run_new_nodes(0, &samples).err().map(|by| Found {
            tx: 0,
            phase: Phase::Build,
            by,
        });
        e
    }

    pub fn run(&mut self, item: Item) {
        if self.found.is_some() {
            return;
        }
        match item {
            Item::Tx(tx) => {
                self.tx += 1;
                if let Err((phase, by)) = self.transaction(tx) {
                    self.found = Some(Found {
                        tx: self.tx,
                        phase,
                        by,
                    });
                }
            }
            Item::Sample(c) => {
                if let Err(by) = self.read(c, false) {
                    self.found = Some(Found {
                        tx: self.tx,
                        phase: Phase::Sample,
                        by,
                    });
                }
            }
        }
    }

    fn read(&self, c: Id, post: bool) -> Result<Id, By> {
        self.g
            .read(c, post, self.brent, Passed::NOTHING, 0)
            .map_err(|f| match f {
                ReadFail::Guard(_) => By::ReadGuard,
                ReadFail::Overflow => By::Overflow,
            })
    }

    fn transaction(&mut self, tx: Tx) -> Result<(), (Phase, By)> {
        for &(h, token) in &tx.sets {
            self.g.set_pending(h, token);
        }
        // Mark.
        if self.design == Design::Grey {
            self.g
                .mark::<true>(&tx.fires)
                .map_err(|_| (Phase::Mark, By::GreyMark))?;
        } else {
            self.g
                .mark::<false>(&tx.fires)
                .expect("the plain mark reports nothing");
        }
        let marked: Vec<Id> = self.g.order.clone();
        let mut relinks = self.g.relinks.clone();
        // Evaluate. Only the reads matter to the question.
        for &n in marked.iter().rev() {
            let post = match self.g.kind(n) {
                Kind::Snapshot => false,
                Kind::Steps => true,
                _ => continue,
            };
            let c = self.g.reads[n as usize];
            self.read(c, post).map_err(|by| (Phase::Evaluate, by))?;
        }
        if let Some(f) = tx.construct {
            let first = self.g.len();
            let samples = f(&mut self.g);
            self.run_new_nodes(first, &samples)
                .map_err(|by| (Phase::Construct, by))?;
        }
        // Commit, then relink in two passes: every queued switch reads its
        // outer's committed selection and moves, then the moves are
        // checked on the graph they made together (F46).
        self.g.commit_holds();
        relinks.sort_unstable();
        relinks.dedup();
        if self.reverse_relinks {
            relinks.reverse();
        }
        let mut moved = Vec::new();
        for &s in &relinks {
            let outer = self.g.outer[s as usize];
            let inner = self.read(outer, false).map_err(|by| (Phase::Relink, by))?;
            if inner != self.g.linked(s) {
                self.g.link(s, inner);
                if self.design == Design::WalkEachMove && self.g.upstream_reaches(inner, s) {
                    return Err((Phase::Relink, By::Walk));
                }
                moved.push(s);
            }
        }
        if self.design == Design::Walk {
            for &s in &moved {
                let inner = self.g.linked(s);
                if self.g.upstream_reaches(inner, s) {
                    return Err((Phase::Relink, By::Walk));
                }
            }
        }
        // Dispatch: a cell listener reads the committed value.
        for &c in &tx.listen {
            if marked.contains(&c) {
                self.read(c, false).map_err(|by| (Phase::Dispatch, by))?;
            }
        }
        Ok(())
    }

    /// The closure's samples, then a pull of every node from `first` on,
    /// in creation order.
    fn run_new_nodes(&mut self, first: Id, samples: &[Id]) -> Result<(), By> {
        for &c in samples {
            self.read(c, false)?;
        }
        self.g.pull_epoch += 1;
        for n in first..self.g.len() {
            self.ensure(n, first)?;
        }
        Ok(())
    }

    /// Memoized pull over the nodes built this instant, with an in-progress
    /// stamp; older nodes are settled already. A switch links at its first
    /// evaluation, to what its outer selected before the instant, and the
    /// walking designs walk upstream from it there; then it runs its inner
    /// at this instant (F48), so it pulls the inner.
    fn ensure(&mut self, n: Id, first: Id) -> Result<(), By> {
        if n < first {
            return Ok(());
        }
        let i = n as usize;
        let busy = 2 * self.g.pull_epoch;
        match self.g.pull[i] {
            s if s == busy + 1 => return Ok(()),
            s if s == busy => return Err(By::PullGuard),
            _ => {}
        }
        self.g.pull[i] = busy;
        let kind = self.g.kind(n);
        let unlinked_switch =
            matches!(kind, Kind::SwitchCell | Kind::SwitchStream) && self.g.linked(n) == NONE;
        if unlinked_switch {
            // A switch_cell depends on its outer; a switch_stream only
            // watches it. Either links to what the outer selected before
            // the instant, which needs no pull (R8).
            let outer = self.g.outer[i];
            if kind == Kind::SwitchCell {
                self.ensure(outer, first)?;
            }
            let inner = self.read(outer, false)?;
            self.g.link(n, inner);
            if self.design != Design::Grey && self.g.upstream_reaches(inner, n) {
                return Err(By::Walk);
            }
            self.ensure(inner, first)?;
        } else {
            for k in 0..self.g.deps[i].len() {
                let d = self.g.deps[i][k];
                self.ensure(d, first)?;
            }
            match kind {
                Kind::Snapshot => {
                    self.read(self.g.reads[i], false)?;
                }
                Kind::Steps => {
                    self.read(self.g.reads[i], true)?;
                }
                _ => {}
            }
        }
        self.g.pull[i] = busy + 1;
        Ok(())
    }
}

/// F50's shape: input `source` feeds `upstream` nodes, each depending on
/// the one before and on one earlier node picked by a fixed PRNG, so a mark
/// over them revisits nodes and the grey check has something to compare.
/// The last is `inner`. A `switch_cell` over a hold of a selector input is
/// linked to a constant, and `downstream` nodes hang off it. Moving it to
/// `inner` is the move F50 timed, and it is acyclic.
pub struct F50 {
    pub g: Graph,
    pub source: Id,
    pub switch: Id,
    pub inner: Id,
}

impl F50 {
    pub fn new(upstream: u32, downstream: u32) -> F50 {
        let mut g = Graph::default();
        let source = g.input();
        let selector = g.input();
        let first = g.stream(&[source]);
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        for i in 1..upstream {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let other = first + (seed % i as u64) as Id;
            let prev = first + i - 1;
            if other == prev {
                g.stream(&[prev]);
            } else {
                g.stream(&[prev, other]);
            }
        }
        let inner = first + upstream - 1;
        let old = g.constant(NONE);
        let select = g.stream(&[selector]);
        let outer = g.hold(select, old);
        let switch = g.switch_cell(outer);
        g.link(switch, old);
        let mut prev = switch;
        for _ in 0..downstream {
            prev = g.stream(&[prev]);
        }
        F50 {
            g,
            source,
            switch,
            inner,
        }
    }

    /// The same graph with the switch already moved to `inner`.
    pub fn moved(upstream: u32, downstream: u32) -> F50 {
        let mut f = F50::new(upstream, downstream);
        f.g.link(f.switch, f.inner);
        f
    }

    /// Runs a walk and a mark once, so the vectors they reuse have grown,
    /// as they have in an engine past its first transactions.
    pub fn warm(&mut self) {
        let (inner, switch) = (self.inner, self.switch);
        self.g.upstream_reaches(inner, switch);
        let _ = self.mark::<true>();
    }

    /// One move checked as RFD 5 checks it: link, then walk upstream from
    /// the new inner. Returns whether it found a cycle.
    pub fn move_and_walk(&mut self) -> bool {
        self.g.link(self.switch, self.inner);
        self.g.upstream_reaches(self.inner, self.switch)
    }

    /// One transaction's mark from `source`, over the moved graph.
    pub fn mark<const GREY: bool>(&mut self) -> Result<(), Id> {
        let source = self.source;
        self.g.mark::<GREY>(&[source])
    }
}
