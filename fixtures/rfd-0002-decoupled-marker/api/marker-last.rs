//! The marker-last design: marker-close, with type-state on `Build`
//! counting open loops, so that a close that leaves no loop open retires
//! every forward built so far.
//!
//! A forward is `Instantaneous<E>`, where `E` is the build's epoch, a
//! type-level number. `Build<E, N>` counts the open loops in `N`; opening
//! a loop takes the build by value and returns `Build<E, S<N>>`, and a
//! close returns `Build<E, N>`, or `Build<S<E>, Z>` when it closed the
//! last open loop: a new epoch. `close` accepts a definition whose marks
//! are all from earlier epochs, so every token built from a forward
//! before the last close counts as `Decoupled` from then on, whatever it
//! is: the loop's own token, a hold built from the forward, a chain.
//! Nothing is re-marked in place; the check compares epochs.
//!
//! Why it is sound: when no loop is open, every forward built so far
//! belongs to a closed loop, each close checked its definition against
//! the loops open in its epoch, and what a closed loop's node reaches is
//! fixed but through switches. A loop opened later is newer than every
//! such token, so no token of an earlier epoch reaches its forward. This
//! assumes the count is honest: one `Build` live at a time, with its loops
//! closed on it. Two `Build`s can trade loops and desynchronise it.
//!
//! A `construct` closure gets a child build one epoch on. Bough runs it at
//! an event, after the declaring scope ended with every loop closed
//! (RFD 2), so every token the closure captured is from a closed epoch.
//! It must hand the child build back with no loop open.
//!
//! Plain operations take `&mut Graph`, which `&mut b` coerces to; only
//! loops and `construct` need the build's type.

#![allow(dead_code)]

use std::any::Any;
use std::marker::PhantomData;

// ----- the mark -----

/// Does not depend on any loop's forward reference this instant.
pub struct Decoupled;

/// Depends on the forward reference of a loop opened in epoch `E`.
pub struct Instantaneous<E>(PhantomData<E>);

/// Depends on both marks' forwards: `merge`'s join of two marks where
/// one isn't `Decoupled`. It keeps both epochs rather than their maximum.
pub struct Both<L, R>(PhantomData<(L, R)>);

/// A decoupledness mark. `Or` is the join `merge` needs; a generic
/// associated type keeps the join out of every `where` clause.
pub trait Mark: 'static {
    type Or<R: Mark>: Mark;
}

impl Mark for Decoupled {
    type Or<R: Mark> = R;
}

impl<E: 'static> Mark for Instantaneous<E> {
    type Or<R: Mark> = Both<Self, R>;
}

impl<L: Mark, R: Mark> Mark for Both<L, R> {
    type Or<O: Mark> = Both<Self, O>;
}

// ----- epochs and the open-loop count -----

/// Type-level zero: the first epoch, and no loop open.
pub struct Z;

/// Type-level successor.
pub struct S<N>(PhantomData<N>);

/// `Self < E`: an epoch that ended before `E` began.
#[diagnostic::on_unimplemented(
    message = "this loop's definition depends on a forward reference of the current epoch",
    label = "reaches the forward of a loop opened since the last close that left none open",
    note = "only `snapshot`, `gate` and `sample` of a cell and a `switch_stream`'s selection read from before the instant, and only `split` and `defer` move to a child instant; a hold delays those reads, not its `steps`; a forward counts as closed once a close leaves no loop open"
)]
pub trait Before<E> {}

impl<N> Before<S<N>> for Z {}

impl<A: Before<B>, B> Before<S<B>> for S<A> {}

/// The build's epoch after a close that leaves `Self` loops open: the
/// next epoch if none is, the same one otherwise.
pub trait Open {
    type After<E>;
}

impl Open for Z {
    type After<E> = S<E>;
}

impl<N> Open for S<N> {
    type After<E> = E;
}

/// What `close` requires of its definition in epoch `E`: no forward of
/// `E`, so no forward of a loop still open.
#[diagnostic::on_unimplemented(
    message = "this loop's definition depends on a forward reference of the current epoch",
    label = "reaches the forward of a loop opened since the last close that left none open",
    note = "only `snapshot`, `gate` and `sample` of a cell and a `switch_stream`'s selection read from before the instant, and only `split` and `defer` move to a child instant; a hold delays those reads, not its `steps`; a forward counts as closed once a close leaves no loop open"
)]
pub trait IsDecoupled<E>: Mark {}

impl<E> IsDecoupled<E> for Decoupled {}

impl<F: Before<E> + 'static, E> IsDecoupled<E> for Instantaneous<F> {}

impl<L: IsDecoupled<E>, R: IsDecoupled<E>, E> IsDecoupled<E> for Both<L, R> {}

// ----- the engine skeleton -----

type Pull = Box<dyn FnMut(&Cx) -> Option<Box<dyn Any>>>;

/// What a node's pull reads: this instant's events, and cells as they were
/// before it.
pub struct Cx {
    events: Vec<Option<Box<dyn Any>>>,
    cells: Vec<Option<Box<dyn Any>>>,
}

impl Cx {
    fn event<A: Clone + 'static>(&self, id: usize) -> Option<A> {
        self.events[id].as_ref()?.downcast_ref::<A>().cloned()
    }

    fn cell<A: 'static>(&self, id: usize) -> &A {
        self.cells[id]
            .as_ref()
            .and_then(|value| value.downcast_ref())
            .expect("a cell has a value")
    }
}

/// The graph under construction. Every operation but a loop's and
/// `construct` takes this, and `&mut b` coerces to it from any `Build`.
#[derive(Default)]
pub struct Graph {
    pulls: Vec<Option<Pull>>,
    cells: Vec<Option<Box<dyn Any>>>,
    /// `depends` declarations: what each node keeps alive (RFD 3).
    reach: Vec<(usize, usize)>,
}

impl Graph {
    fn node(&mut self, pull: Option<Pull>, value: Option<Box<dyn Any>>) -> usize {
        self.pulls.push(pull);
        self.cells.push(value);
        self.pulls.len() - 1
    }

    fn materialize<S: Source>(&mut self, mut chain: S) -> Pull {
        Box::new(move |cx| chain.pull(cx).map(|event| Box::new(event) as Box<dyn Any>))
    }

    /// A `split` or `defer`: two nodes, one that takes the chain's event at
    /// t, and one that emits in t's children. The second has no pull, so
    /// it depends on nothing in the instant it fires in.
    fn child_instant<S: Source>(&mut self, chain: S) -> usize {
        let take = self.materialize(chain);
        self.node(Some(take), None);
        self.node(None, None)
    }

    /// Declares that `node` keeps `on` alive, as for the tokens a
    /// `construct` closure captures. A reach declaration for collection
    /// (RFD 3), not a dependency, so it reads no mark and changes none:
    /// it returns nothing and takes its tokens by reference.
    pub fn depends(&mut self, node: &dyn Trace, on: &[&dyn Trace]) {
        let node = node.node();
        self.reach.extend(on.iter().map(|value| (node, value.node())));
    }

    pub fn input<A: 'static>(&mut self) -> Stream<A> {
        Stream::new(self.node(None, None))
    }

    pub fn constant<A: 'static>(&mut self, value: A) -> Cell<A> {
        Cell::new(self.node(None, Some(Box::new(value))))
    }
}

/// A build in epoch `E` with `N` loops open. Loops take and return it by
/// value, since they change its type.
pub struct Build<E = Z, N = Z> {
    graph: Graph,
    state: PhantomData<fn() -> (E, N)>,
}

impl Build {
    pub fn new() -> Self {
        Build {
            graph: Graph::default(),
            state: PhantomData,
        }
    }
}

impl Default for Build {
    fn default() -> Self {
        Self::new()
    }
}

impl<E, N> std::ops::Deref for Build<E, N> {
    type Target = Graph;

    fn deref(&self) -> &Graph {
        &self.graph
    }
}

impl<E, N> std::ops::DerefMut for Build<E, N> {
    fn deref_mut(&mut self) -> &mut Graph {
        &mut self.graph
    }
}

impl<E: 'static, N> Build<E, N> {
    fn retype<F, M>(self) -> Build<F, M> {
        Build {
            graph: self.graph,
            state: PhantomData,
        }
    }

    /// The forward is `Instantaneous<E>` for good; it counts as closed
    /// once the epoch has moved on.
    pub fn cell_loop<A: 'static>(
        mut self,
    ) -> (Build<E, S<N>>, Cell<A, Instantaneous<E>>, CellLoop<A, E>) {
        let id = self.graph.node(None, None);
        (
            self.retype(),
            Cell::new(id),
            CellLoop {
                id,
                event: PhantomData,
            },
        )
    }

    pub fn stream_loop<A: 'static>(
        mut self,
    ) -> (Build<E, S<N>>, Stream<A, Instantaneous<E>>, StreamLoop<A, E>) {
        let id = self.graph.node(None, None);
        (
            self.retype(),
            Stream::new(id),
            StreamLoop {
                id,
                event: PhantomData,
            },
        )
    }
}

// ----- tokens -----

/// A stream node. Linear, as in Bough.
pub struct Stream<A, M = Decoupled> {
    id: usize,
    token: PhantomData<fn() -> (A, M)>,
}

impl<A, M> Stream<A, M> {
    fn new(id: usize) -> Self {
        Self {
            id,
            token: PhantomData,
        }
    }
}

/// A cell node. A token of one integer, so `Copy`.
pub struct Cell<A, M = Decoupled> {
    id: usize,
    token: PhantomData<fn() -> (A, M)>,
}

impl<A, M> Clone for Cell<A, M> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<A, M> Copy for Cell<A, M> {}

impl<A, M> Cell<A, M> {
    fn new(id: usize) -> Self {
        Self {
            id,
            token: PhantomData,
        }
    }
}

impl<A: 'static, M: Mark> Cell<A, M> {
    /// The steps view carries the cell's mark: a step is this instant's.
    pub fn steps(self) -> Stream<A, M> {
        Stream::new(self.id)
    }

    /// Reads the value from before the instant, as a value: nothing
    /// carries the mark into what is built from it, as `snapshot` drops it.
    /// Sampling a loop's forward before it closes is a build-time panic in
    /// Bough, so this is for `construct` closures, which run later.
    pub fn sample(self, b: &Graph) -> &A {
        b.cells[self.id]
            .as_ref()
            .and_then(|value| value.downcast_ref())
            .expect("a closed loop's cell has a value")
    }

    /// A read-through cell, computed this instant, so it keeps the mark.
    pub fn map_cell<B: 'static>(self, b: &mut Graph, f: impl Fn(&A) -> B + 'static) -> Cell<B, M> {
        let pull: Pull = Box::new(move |cx| Some(Box::new(f(cx.cell::<A>(self.id)))));
        Cell::new(b.node(Some(pull), None))
    }

    /// Builds graph from the cell's value, as Bough's `construct` does.
    /// The closure gets a child build an epoch on and hands it back with
    /// no loop open.
    pub fn construct<B: 'static, E, N, E2>(
        self,
        b: &mut Build<E, N>,
        f: impl Fn(Build<S<E>>, &A) -> (Build<E2>, B) + 'static,
    ) -> Cell<B, M> {
        let _ = f;
        Cell::new(b.node(None, None))
    }
}

impl<A: 'static, MI: Mark, MO: Mark> Cell<Stream<A, MI>, MO> {
    /// The selection is read from before the instant, so the outer's mark
    /// is dropped; the output is the inner's this instant, so it keeps the
    /// inner's. Every inner has the cell's value type, so every candidate
    /// carries the same static mark.
    pub fn switch_stream(self, b: &mut Graph) -> Stream<A, MI> {
        Stream::new(b.node(None, None))
    }
}

/// A token `depends` can name.
pub trait Trace {
    fn node(&self) -> usize;
}

impl<A, M> Trace for Stream<A, M> {
    fn node(&self) -> usize {
        self.id
    }
}

impl<A, M> Trace for Cell<A, M> {
    fn node(&self) -> usize {
        self.id
    }
}

// ----- chains -----

pub trait Source: Sized + 'static {
    type Event: 'static;
    type Mark: Mark;

    fn pull(&mut self, cx: &Cx) -> Option<Self::Event>;

    fn map<B, F: FnMut(Self::Event) -> B + 'static>(self, f: F) -> Map<Self, F> {
        Map { source: self, f }
    }

    fn filter<P: FnMut(&Self::Event) -> bool + 'static>(self, predicate: P) -> Filter<Self, P> {
        Filter {
            source: self,
            predicate,
        }
    }

    /// Reads the cell from before the instant, so its mark doesn't matter.
    fn snapshot<C: 'static, CM, B, F>(self, cell: Cell<C, CM>, f: F) -> Snapshot<Self, C, F>
    where
        F: FnMut(Self::Event, &C) -> B + 'static,
    {
        Snapshot {
            source: self,
            cell: cell.id,
            value: PhantomData,
            f,
        }
    }

    /// Keeps the events during which the cell was `true`. Reads the cell
    /// from before the instant, as `snapshot` does, so the cell's mark
    /// doesn't matter; the stream's passes through.
    fn gate<CM>(self, cell: Cell<bool, CM>) -> Gate<Self> {
        Gate {
            source: self,
            cell: cell.id,
        }
    }

    /// The semantics' `Execute`: runs `f` at each event with a build
    /// context. Bough's `construct` on a stream. The output fires in the
    /// instant the input does, so it keeps the input's mark; what `f`
    /// builds and captures is not in its type.
    /// The closure gets a child build an epoch on, as `Cell::construct`.
    fn construct<B, E, N, E2, F>(self, b: &mut Build<E, N>, f: F) -> Stream<B, Self::Mark>
    where
        B: 'static,
        F: FnMut(Build<S<E>>, Self::Event) -> (Build<E2>, B) + 'static,
    {
        let _ = f;
        let pull = b.materialize(self);
        Stream::new(b.node(Some(pull), None))
    }

    /// Emits each item of each event in a child instant of its own,
    /// `t ++ [n]`. The output depends on nothing in the instant it fires
    /// in, the parent's or a child's, so it starts with no mark: the
    /// input's is dropped.
    fn split(self, b: &mut Graph) -> Stream<<Self::Event as IntoIterator>::Item>
    where
        Self::Event: IntoIterator,
        <Self::Event as IntoIterator>::Item: 'static,
    {
        Stream::new(b.child_instant(self))
    }

    /// Emits each event again in t's first child instant, `t ++ [0]`, a
    /// split of one. Its output starts with no mark, as `split`'s does.
    fn defer(self, b: &mut Graph) -> Stream<Self::Event> {
        Stream::new(b.child_instant(self))
    }

    fn hold(self, b: &mut Graph, init: Self::Event) -> Cell<Self::Event, Self::Mark> {
        let pull = b.materialize(self);
        Cell::new(b.node(Some(pull), Some(Box::new(init))))
    }

    fn merge<O>(
        self,
        b: &mut Graph,
        other: O,
    ) -> Stream<Self::Event, <Self::Mark as Mark>::Or<O::Mark>>
    where
        O: Source<Event = Self::Event>,
    {
        let mut left = self;
        let mut right = other;
        let pull: Pull = Box::new(move |cx| {
            let event = left.pull(cx).or_else(|| right.pull(cx))?;
            Some(Box::new(event))
        });
        Stream::new(b.node(Some(pull), None))
    }
}

impl<A: Clone + 'static, M: Mark> Source for Stream<A, M> {
    type Event = A;
    type Mark = M;

    fn pull(&mut self, cx: &Cx) -> Option<A> {
        cx.event(self.id)
    }
}

pub struct Map<S, F> {
    source: S,
    f: F,
}

impl<S: Source, B: 'static, F: FnMut(S::Event) -> B + 'static> Source for Map<S, F> {
    type Event = B;
    type Mark = S::Mark;

    fn pull(&mut self, cx: &Cx) -> Option<B> {
        self.source.pull(cx).map(&mut self.f)
    }
}

pub struct Filter<S, P> {
    source: S,
    predicate: P,
}

impl<S: Source, P: FnMut(&S::Event) -> bool + 'static> Source for Filter<S, P> {
    type Event = S::Event;
    type Mark = S::Mark;

    fn pull(&mut self, cx: &Cx) -> Option<S::Event> {
        self.source.pull(cx).filter(&mut self.predicate)
    }
}

pub struct Snapshot<S, C, F> {
    source: S,
    cell: usize,
    value: PhantomData<fn() -> C>,
    f: F,
}

impl<S, C, B, F> Source for Snapshot<S, C, F>
where
    S: Source,
    C: 'static,
    B: 'static,
    F: FnMut(S::Event, &C) -> B + 'static,
{
    type Event = B;
    type Mark = S::Mark;

    fn pull(&mut self, cx: &Cx) -> Option<B> {
        let event = self.source.pull(cx)?;
        Some((self.f)(event, cx.cell::<C>(self.cell)))
    }
}

pub struct Gate<S> {
    source: S,
    cell: usize,
}

impl<S: Source> Source for Gate<S> {
    type Event = S::Event;
    type Mark = S::Mark;

    fn pull(&mut self, cx: &Cx) -> Option<S::Event> {
        let event = self.source.pull(cx)?;
        cx.cell::<bool>(self.cell).then_some(event)
    }
}

// ----- loops -----

pub struct CellLoop<A, E> {
    id: usize,
    event: PhantomData<fn() -> (A, E)>,
}

impl<A: 'static, E: 'static> CellLoop<A, E> {
    /// Closes the loop in the epoch it was opened in, against the forwards
    /// of that epoch, and returns the build with one loop fewer, in the
    /// next epoch if none is left, and the loop's cell, `Decoupled`, as
    /// marker-close does.
    pub fn close<N: Open, M: IsDecoupled<E>>(
        self,
        mut b: Build<E, S<N>>,
        definition: Cell<A, M>,
    ) -> (Build<N::After<E>, N>, Cell<A>) {
        b.graph.pulls[self.id] = b.graph.pulls[definition.id].take();
        (b.retype(), Cell::new(self.id))
    }
}

pub struct StreamLoop<A, E> {
    id: usize,
    event: PhantomData<fn() -> (A, E)>,
}

impl<A: 'static, E: 'static> StreamLoop<A, E> {
    pub fn close<N: Open, D>(
        self,
        mut b: Build<E, S<N>>,
        definition: D,
    ) -> (Build<N::After<E>, N>, Stream<A>)
    where
        D: Source<Event = A>,
        D::Mark: IsDecoupled<E>,
    {
        let pull = b.graph.materialize(definition);
        b.graph.pulls[self.id] = Some(pull);
        (b.retype(), Stream::new(self.id))
    }
}
