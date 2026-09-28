//! The marker design: Keating and Gale's decoupledness bit on every stream
//! and cell type, after Sculthorpe and Nilsson's decoupledness-indexed
//! signal functions.
//!
//! A value's mark says whether it depends on an open loop's forward
//! reference in the same instant. The forward reference is
//! `Instantaneous`; everything else starts `Decoupled`. Adapters and
//! materializers pass the mark on, `merge` joins two marks, and a read of a
//! cell from before the instant (`snapshot`, and a `switch_stream`'s
//! selection) ignores the cell's mark. A hold's mark is its input's: a hold
//! delays reads made before the instant, not its steps view, so `steps`
//! gives the mark back. `close` requires a `Decoupled` definition.
//!
//! Streams are fused chains as in RFD 4: `map`, `filter` and `snapshot` are
//! adapter types, and the mark is an associated type of `Source`, so it
//! adds no type parameter to a chain. Tokens are erased node handles, so
//! they carry the mark as a parameter, defaulting to `Decoupled`. The
//! engine is only the skeleton that makes materializers box their chains,
//! so a build monomorphizes each chain as the real one would. It never
//! runs.

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
    note = "only `snapshot` of a cell and a `switch_stream`'s selection read from before the instant; a hold delays those reads, not its `steps`"
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

    pub fn input<A: 'static>(&mut self) -> Stream<A> {
        Stream::new(self.node(None, None))
    }

    pub fn constant<A: 'static>(&mut self, value: A) -> Cell<A> {
        Cell::new(self.node(None, Some(Box::new(value))))
    }

    /// The forward is `Instantaneous` for good: its type can't learn that
    /// the loop has closed.
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

// ----- loops -----

pub struct CellLoop<A> {
    id: usize,
    event: PhantomData<fn() -> A>,
}

impl<A: 'static> CellLoop<A> {
    pub fn close<M: IsDecoupled>(self, b: &mut Build, definition: Cell<A, M>) {
        b.pulls[self.id] = b.pulls[definition.id].take();
    }
}

pub struct StreamLoop<A> {
    id: usize,
    event: PhantomData<fn() -> A>,
}

impl<A: 'static> StreamLoop<A> {
    pub fn close<S>(self, b: &mut Build, definition: S)
    where
        S: Source<Event = A>,
        S::Mark: IsDecoupled,
    {
        let pull = b.materialize(definition);
        b.pulls[self.id] = Some(pull);
    }
}
