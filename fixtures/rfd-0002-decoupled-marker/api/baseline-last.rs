//! The unmarked API the marker-last fixtures check against: the
//! baseline-close design, with loops taking and returning the build by
//! value as `marker-last.rs` does, but no type-state on it. Every such
//! fixture builds here; one that fails is broken, not refused.

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
    /// (RFD 3), not a dependency.
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

/// The build, by value, as marker-last's, with nothing in its type.
#[derive(Default)]
pub struct Build {
    graph: Graph,
}

impl Build {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cell_loop<A: 'static>(mut self) -> (Build, Cell<A>, CellLoop<A>) {
        let id = self.graph.node(None, None);
        (
            self,
            Cell::new(id),
            CellLoop {
                id,
                event: PhantomData,
            },
        )
    }

    pub fn stream_loop<A: 'static>(mut self) -> (Build, Stream<A>, StreamLoop<A>) {
        let id = self.graph.node(None, None);
        (
            self,
            Stream::new(id),
            StreamLoop {
                id,
                event: PhantomData,
            },
        )
    }
}

impl std::ops::Deref for Build {
    type Target = Graph;

    fn deref(&self) -> &Graph {
        &self.graph
    }
}

impl std::ops::DerefMut for Build {
    fn deref_mut(&mut self) -> &mut Graph {
        &mut self.graph
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

    /// Reads the value from before the instant. Sampling a loop's
    /// forward before it closes is a build-time panic in Bough, so this is
    /// for `construct` closures, which run later.
    pub fn sample(self, b: &Graph) -> &A {
        b.cells[self.id]
            .as_ref()
            .and_then(|value| value.downcast_ref())
            .expect("a closed loop's cell has a value")
    }

    /// A read-through cell.
    pub fn map_cell<B: 'static>(self, b: &mut Graph, f: impl Fn(&A) -> B + 'static) -> Cell<B> {
        let pull: Pull = Box::new(move |cx| Some(Box::new(f(cx.cell::<A>(self.id)))));
        Cell::new(b.node(Some(pull), None))
    }

    /// Builds graph from the cell's value, as Bough's `construct` does.
    pub fn construct<B: 'static>(
        self,
        b: &mut Build,
        f: impl Fn(Build, &A) -> (Build, B) + 'static,
    ) -> Cell<B> {
        let _ = f;
        Cell::new(b.node(None, None))
    }
}

impl<A: 'static> Cell<Stream<A>> {
    pub fn switch_stream(self, b: &mut Graph) -> Stream<A> {
        Stream::new(b.node(None, None))
    }
}

/// A token `depends` can name.
pub trait Trace {
    fn node(&self) -> usize;
}

impl<A> Trace for Stream<A> {
    fn node(&self) -> usize {
        self.id
    }
}

impl<A> Trace for Cell<A> {
    fn node(&self) -> usize {
        self.id
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

    /// Keeps the events during which the cell was `true`.
    fn gate(self, cell: Cell<bool>) -> Gate<Self> {
        Gate {
            source: self,
            cell: cell.id,
        }
    }

    /// The semantics' `Execute`: runs `f` at each event with a build
    /// context. Bough's `construct` on a stream.
    fn construct<B, F>(self, b: &mut Build, f: F) -> Stream<B>
    where
        B: 'static,
        F: FnMut(Build, Self::Event) -> (Build, B) + 'static,
    {
        let _ = f;
        let pull = b.materialize(self);
        Stream::new(b.node(Some(pull), None))
    }

    /// Emits each item of each event in a child instant of its own.
    fn split(self, b: &mut Graph) -> Stream<<Self::Event as IntoIterator>::Item>
    where
        Self::Event: IntoIterator,
        <Self::Event as IntoIterator>::Item: 'static,
    {
        Stream::new(b.child_instant(self))
    }

    /// Emits each event again in the first child instant.
    fn defer(self, b: &mut Graph) -> Stream<Self::Event> {
        Stream::new(b.child_instant(self))
    }

    fn hold(self, b: &mut Graph, init: Self::Event) -> Cell<Self::Event> {
        let pull = b.materialize(self);
        Cell::new(b.node(Some(pull), Some(Box::new(init))))
    }

    fn merge<O>(self, b: &mut Graph, other: O) -> Stream<Self::Event>
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

pub struct Gate<S> {
    source: S,
    cell: usize,
}

impl<S: Source> Source for Gate<S> {
    type Event = S::Event;

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
    pub fn close(self, mut b: Build, definition: Cell<A>) -> (Build, Cell<A>) {
        b.graph.pulls[self.id] = b.graph.pulls[definition.id].take();
        (b, Cell::new(self.id))
    }
}

pub struct StreamLoop<A> {
    id: usize,
    event: PhantomData<fn() -> A>,
}

impl<A: 'static> StreamLoop<A> {
    pub fn close<S>(self, mut b: Build, definition: S) -> (Build, Stream<A>)
    where
        S: Source<Event = A>,
    {
        let pull = b.graph.materialize(definition);
        b.graph.pulls[self.id] = Some(pull);
        (b, Stream::new(self.id))
    }
}
