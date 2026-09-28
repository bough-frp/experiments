//! The marker-close design: the marker design, with `close` returning
//! the loop's own token re-marked `Decoupled`.
//!
//! In the marker design a loop's forward is `Instantaneous` for good, so
//! a second loop that reads the first through its forward after the first
//! has closed is refused. Here `close` hands back a token for the loop's
//! node, `Decoupled`, and code after the close reads that instead:
//! `let block = block_loop.close(b, next_block);` shadows the forward.
//!
//! Why the re-mark is sound: a `Decoupled` token's node reaches no open
//! forward in the same instant, and what it reaches is fixed when it is
//! built, since nodes only point at older nodes and only a loop's node
//! changes its inputs, at its close. `close` requires a `Decoupled`
//! definition, so once closed the loop's node reaches what the definition
//! reaches, which is no open forward, and it keeps that. The one node
//! whose inputs change after it is built is a switch, whose inner moves;
//! that is outside what a mark on a token can see, here as in `marker`.
//!
//! Only the loop's own token is re-marked. A token built from the forward
//! before the close keeps `Instantaneous`: one bit can't say which loops a
//! value depends on, so it can't say that closing this one leaves none
//! open.
//!
//! Everything else is `marker.rs` as it was.

#![allow(dead_code)]

use std::any::Any;
use std::marker::PhantomData;

// ----- the mark -----

/// Does not depend on any open loop's forward reference this instant.
pub struct Decoupled;

/// Depends on some open loop's forward reference this instant.
pub struct Instantaneous;

/// A decoupledness bit. `Or` is the join `merge` needs; a generic
/// associated type keeps the join out of every `where` clause.
pub trait Mark: 'static {
    type Or<R: Mark>: Mark;
}

impl Mark for Decoupled {
    type Or<R: Mark> = R;
}

impl Mark for Instantaneous {
    type Or<R: Mark> = Instantaneous;
}

/// What `close` requires of its definition.
#[diagnostic::on_unimplemented(
    message = "this loop's definition depends on a loop's forward reference in the same instant",
    label = "reaches a forward reference without a read from before the instant",
    note = "only `snapshot`, `gate` and `sample` of a cell and a `switch_stream`'s selection read from before the instant, and only `split` and `defer` move to a child instant; a hold delays those reads, not its `steps`; a forward stays `Instantaneous` after its loop closes, so read a closed loop through the token its `close` returned"
)]
pub trait IsDecoupled: Mark {}

impl IsDecoupled for Decoupled {}

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

#[derive(Default)]
pub struct Build {
    pulls: Vec<Option<Pull>>,
    cells: Vec<Option<Box<dyn Any>>>,
    /// `depends` declarations: what each node keeps alive (RFD 3).
    reach: Vec<(usize, usize)>,
}

impl Build {
    pub fn new() -> Self {
        Self::default()
    }

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

    /// The forward is `Instantaneous` for good: its type can't learn that
    /// the loop has closed. `close` returns the token to use after it.
    pub fn cell_loop<A: 'static>(&mut self) -> (Cell<A, Instantaneous>, CellLoop<A>) {
        let id = self.node(None, None);
        (
            Cell::new(id),
            CellLoop {
                id,
                event: PhantomData,
            },
        )
    }

    pub fn stream_loop<A: 'static>(&mut self) -> (Stream<A, Instantaneous>, StreamLoop<A>) {
        let id = self.node(None, None);
        (
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
    pub fn sample(self, b: &Build) -> &A {
        b.cells[self.id]
            .as_ref()
            .and_then(|value| value.downcast_ref())
            .expect("a closed loop's cell has a value")
    }

    /// A read-through cell, computed this instant, so it keeps the mark.
    pub fn map_cell<B: 'static>(self, b: &mut Build, f: impl Fn(&A) -> B + 'static) -> Cell<B, M> {
        let pull: Pull = Box::new(move |cx| Some(Box::new(f(cx.cell::<A>(self.id)))));
        Cell::new(b.node(Some(pull), None))
    }

    /// Builds graph from the cell's value, as Bough's `construct` does.
    pub fn construct<B: 'static>(
        self,
        b: &mut Build,
        f: impl Fn(&mut Build, &A) -> B + 'static,
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
    pub fn switch_stream(self, b: &mut Build) -> Stream<A, MI> {
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
    fn construct<B, F>(self, b: &mut Build, f: F) -> Stream<B, Self::Mark>
    where
        B: 'static,
        F: FnMut(&mut Build, Self::Event) -> B + 'static,
    {
        let _ = f;
        let pull = b.materialize(self);
        Stream::new(b.node(Some(pull), None))
    }

    /// Emits each item of each event in a child instant of its own,
    /// `t ++ [n]`. The output depends on nothing in the instant it fires
    /// in, the parent's or a child's, so it starts with no mark: the
    /// input's is dropped.
    fn split(self, b: &mut Build) -> Stream<<Self::Event as IntoIterator>::Item>
    where
        Self::Event: IntoIterator,
        <Self::Event as IntoIterator>::Item: 'static,
    {
        Stream::new(b.child_instant(self))
    }

    /// Emits each event again in t's first child instant, `t ++ [0]`, a
    /// split of one. Its output starts with no mark, as `split`'s does.
    fn defer(self, b: &mut Build) -> Stream<Self::Event> {
        Stream::new(b.child_instant(self))
    }

    fn hold(self, b: &mut Build, init: Self::Event) -> Cell<Self::Event, Self::Mark> {
        let pull = b.materialize(self);
        Cell::new(b.node(Some(pull), Some(Box::new(init))))
    }

    fn merge<O>(
        self,
        b: &mut Build,
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

pub struct CellLoop<A> {
    id: usize,
    event: PhantomData<fn() -> A>,
}

impl<A: 'static> CellLoop<A> {
    /// Returns the loop's cell, `Decoupled`: it now reaches what the
    /// definition reaches, and the definition is `Decoupled`.
    /// Not `#[must_use]`: most closes need nothing after them, and the
    /// lint would fire on each; dropping the token costs nothing until a
    /// later loop reads the forward, which `close` then refuses.
    pub fn close<M: IsDecoupled>(self, b: &mut Build, definition: Cell<A, M>) -> Cell<A> {
        b.pulls[self.id] = b.pulls[definition.id].take();
        Cell::new(self.id)
    }
}

pub struct StreamLoop<A> {
    id: usize,
    event: PhantomData<fn() -> A>,
}

impl<A: 'static> StreamLoop<A> {
    /// Returns a stream token for the loop's node, `Decoupled`. The
    /// forward is linear and usually spent before the close, so this is
    /// the only way to read the loop's stream after it; a second token on
    /// one materialized node is what `steps` gives a cell already.
    pub fn close<S>(self, b: &mut Build, definition: S) -> Stream<A>
    where
        S: Source<Event = A>,
        S::Mark: IsDecoupled,
    {
        let pull = b.materialize(definition);
        b.pulls[self.id] = Some(pull);
        Stream::new(self.id)
    }
}
