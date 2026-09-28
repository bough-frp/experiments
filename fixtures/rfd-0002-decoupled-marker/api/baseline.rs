//! The baseline design: the marker design's API with no mark, as RFD 2
//! has it. Loops are checked at run time, when they close, so every
//! fixture builds here; a fixture that fails here is broken, not refused.
//! Same engine skeleton, same chains, same token shapes.

#![allow(dead_code)]

use std::any::Any;
use std::marker::PhantomData;

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

    pub fn cell_loop<A: 'static>(&mut self) -> (Cell<A>, CellLoop<A>) {
        let id = self.node(None, None);
        (
            Cell::new(id),
            CellLoop {
                id,
                event: PhantomData,
            },
        )
    }

    pub fn stream_loop<A: 'static>(&mut self) -> (Stream<A>, StreamLoop<A>) {
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
pub struct Stream<A> {
    id: usize,
    token: PhantomData<fn() -> A>,
}

impl<A> Stream<A> {
    fn new(id: usize) -> Self {
        Self {
            id,
            token: PhantomData,
        }
    }
}

/// A cell node. A token of one integer, so `Copy`.
pub struct Cell<A> {
    id: usize,
    token: PhantomData<fn() -> A>,
}

impl<A> Clone for Cell<A> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<A> Copy for Cell<A> {}

impl<A> Cell<A> {
    fn new(id: usize) -> Self {
        Self {
            id,
            token: PhantomData,
        }
    }
}

impl<A: 'static> Cell<A> {
    pub fn steps(self) -> Stream<A> {
        Stream::new(self.id)
    }

    /// A read-through cell.
    pub fn map_cell<B: 'static>(self, b: &mut Build, f: impl Fn(&A) -> B + 'static) -> Cell<B> {
        let pull: Pull = Box::new(move |cx| Some(Box::new(f(cx.cell::<A>(self.id)))));
        Cell::new(b.node(Some(pull), None))
    }

    /// Builds graph from the cell's value, as Bough's `construct` does.
    pub fn construct<B: 'static>(
        self,
        b: &mut Build,
        f: impl Fn(&mut Build, &A) -> B + 'static,
    ) -> Cell<B> {
        let _ = f;
        Cell::new(b.node(None, None))
    }
}

impl<A: 'static> Cell<Stream<A>> {
    pub fn switch_stream(self, b: &mut Build) -> Stream<A> {
        Stream::new(b.node(None, None))
    }
}

// ----- chains -----

pub trait Source: Sized + 'static {
    type Event: 'static;

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

    fn snapshot<C: 'static, B, F>(self, cell: Cell<C>, f: F) -> Snapshot<Self, C, F>
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

    fn hold(self, b: &mut Build, init: Self::Event) -> Cell<Self::Event> {
        let pull = b.materialize(self);
        Cell::new(b.node(Some(pull), Some(Box::new(init))))
    }

    fn merge<O>(self, b: &mut Build, other: O) -> Stream<Self::Event>
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

impl<A: Clone + 'static> Source for Stream<A> {
    type Event = A;

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
    pub fn close(self, b: &mut Build, definition: Cell<A>) {
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
    {
        let pull = b.materialize(definition);
        b.pulls[self.id] = Some(pull);
    }
}
