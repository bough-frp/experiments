//! The rows design: the marker design with Cuoq and Pouzet's presence rows
//! in place of the one bit, for the case the bit is too coarse for, two
//! loops where one reads the other's steps.
//!
//! A value's row is the set of loops whose forward reference it depends on
//! this instant. Stable Rust has no row polymorphism and no const-generic
//! arithmetic, so a row is a type-level bitset of four slots, `R<_, _, _,
//! _>` of `B0`/`B1`, and `merge` joins rows bitwise through generic
//! associated types. The user gives each loop a slot, `L0` to `L3`.
//!
//! A forward's type is fixed when the loop is declared, before its
//! definition exists, so it can't learn what the definition depends on.
//! If it carried only its own slot, two loops each reading the other's
//! steps would both close. The declaration therefore also lists the other
//! loops the definition may depend on this instant; the forward's row is
//! its slot and that list, and `close` checks that the definition's row
//! leaves out the loop's own slot and stays within the list.

#![allow(dead_code)]
// A row is a tuple of four bits in a type; that is the design, not noise.
#![allow(clippy::type_complexity)]

use std::any::Any;
use std::marker::PhantomData;

// ----- rows -----

pub struct B0;
pub struct B1;

pub trait Bit: 'static {
    type Or<X: Bit>: Bit;
}

impl Bit for B0 {
    type Or<X: Bit> = X;
}

impl Bit for B1 {
    type Or<X: Bit> = B1;
}

/// A row of four slots.
pub struct R<A, B, C, D>(PhantomData<fn() -> (A, B, C, D)>);

pub type Empty = R<B0, B0, B0, B0>;

/// The join `merge` needs, as in the marker design.
pub trait Row: 'static {
    type S0: Bit;
    type S1: Bit;
    type S2: Bit;
    type S3: Bit;
    type Or<X: Row>: Row;
}

impl<A: Bit, B: Bit, C: Bit, D: Bit> Row for R<A, B, C, D> {
    type S0 = A;
    type S1 = B;
    type S2 = C;
    type S3 = D;
    type Or<X: Row> = R<A::Or<X::S0>, B::Or<X::S1>, C::Or<X::S2>, D::Or<X::S3>>;
}

/// A loop's slot.
pub trait Slot: 'static {
    type Only: Row;
}

pub struct L0;
pub struct L1;
pub struct L2;
pub struct L3;

impl Slot for L0 {
    type Only = R<B1, B0, B0, B0>;
}

impl Slot for L1 {
    type Only = R<B0, B1, B0, B0>;
}

impl Slot for L2 {
    type Only = R<B0, B0, B1, B0>;
}

impl Slot for L3 {
    type Only = R<B0, B0, B0, B1>;
}

/// The row of one slot, for declarations.
pub type Only<S> = <S as Slot>::Only;

/// The union of two rows, for declarations.
pub type Union<X, Y> = <X as Row>::Or<Y>;

/// What `close` requires first: the definition doesn't depend on this
/// loop's own forward this instant.
#[diagnostic::on_unimplemented(
    message = "this loop's definition depends on its own forward reference in the same instant",
    label = "reaches the forward reference without a read from before the instant",
    note = "only `snapshot` of a cell and a `switch_stream`'s selection read from before the instant; a hold delays those reads, not its `steps`"
)]
pub trait Absent<S: Slot>: Row {}

impl<B: Bit, C: Bit, D: Bit> Absent<L0> for R<B0, B, C, D> {}
impl<A: Bit, C: Bit, D: Bit> Absent<L1> for R<A, B0, C, D> {}
impl<A: Bit, B: Bit, D: Bit> Absent<L2> for R<A, B, B0, D> {}
impl<A: Bit, B: Bit, C: Bit> Absent<L3> for R<A, B, C, B0> {}

/// One slot of `Within`, which carries the message because it is the
/// bound that fails.
#[diagnostic::on_unimplemented(
    message = "this loop's definition depends in the same instant on a loop its declaration doesn't list",
    label = "depends on a loop outside the declared row",
    note = "list that loop in the declaration's row, or read it through `snapshot`"
)]
pub trait BitWithin<X: Bit>: Bit {}

impl<X: Bit> BitWithin<X> for B0 {}
impl BitWithin<B1> for B1 {}

/// What `close` requires second: the definition depends this instant only
/// on loops its declaration lists, so the forward's row, fixed at
/// declaration, covers it.
#[diagnostic::on_unimplemented(
    message = "this loop's definition depends in the same instant on a loop its declaration doesn't list",
    label = "depends on a loop outside the declared row",
    note = "list that loop in the declaration's row, or read it through `snapshot`"
)]
pub trait Within<Declared: Row>: Row {}

impl<A, B, C, D, W, X, Y, Z> Within<R<W, X, Y, Z>> for R<A, B, C, D>
where
    A: BitWithin<W>,
    B: BitWithin<X>,
    C: BitWithin<Y>,
    D: BitWithin<Z>,
    W: Bit,
    X: Bit,
    Y: Bit,
    Z: Bit,
{
}

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

    /// Declares loop `S`, whose definition may depend this instant on the
    /// loops in `Deps`. The forward's row is `S` and `Deps`.
    pub fn cell_loop<A: 'static, S: Slot, Deps: Row>(
        &mut self,
    ) -> (Cell<A, Union<Only<S>, Deps>>, CellLoop<A, S, Deps>) {
        let id = self.node(None, None);
        (
            Cell::new(id),
            CellLoop {
                id,
                event: PhantomData,
            },
        )
    }

    pub fn stream_loop<A: 'static, S: Slot, Deps: Row>(
        &mut self,
    ) -> (Stream<A, Union<Only<S>, Deps>>, StreamLoop<A, S, Deps>) {
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
pub struct Stream<A, M = Empty> {
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
pub struct Cell<A, M = Empty> {
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

impl<A: 'static, M: Row> Cell<A, M> {
    /// The steps view carries the cell's row: a step is this instant's.
    pub fn steps(self) -> Stream<A, M> {
        Stream::new(self.id)
    }

    /// A read-through cell, computed this instant, so it keeps the row.
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

impl<A: 'static, MI: Row, MO: Row> Cell<Stream<A, MI>, MO> {
    /// The selection is read from before the instant, so the outer's row
    /// is dropped; the output is the inner's this instant, so it keeps the
    /// inner's. Every inner has the cell's value type, so every candidate
    /// carries the same static row.
    pub fn switch_stream(self, b: &mut Build) -> Stream<A, MI> {
        Stream::new(b.node(None, None))
    }
}

// ----- chains -----

pub trait Source: Sized + 'static {
    type Event: 'static;
    type Mark: Row;

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

    /// Reads the cell from before the instant, so its row doesn't matter.
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
    ) -> Stream<Self::Event, <Self::Mark as Row>::Or<O::Mark>>
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

impl<A: Clone + 'static, M: Row> Source for Stream<A, M> {
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

pub struct CellLoop<A, S, Deps> {
    id: usize,
    event: PhantomData<fn() -> (A, S, Deps)>,
}

impl<A: 'static, S: Slot, Deps: Row> CellLoop<A, S, Deps> {
    pub fn close<M: Absent<S> + Within<Deps>>(self, b: &mut Build, definition: Cell<A, M>) {
        b.pulls[self.id] = b.pulls[definition.id].take();
    }
}

pub struct StreamLoop<A, S, Deps> {
    id: usize,
    event: PhantomData<fn() -> (A, S, Deps)>,
}

impl<A: 'static, S: Slot, Deps: Row> StreamLoop<A, S, Deps> {
    pub fn close<Def>(self, b: &mut Build, definition: Def)
    where
        Def: Source<Event = A>,
        Def::Mark: Absent<S> + Within<Deps>,
    {
        let pull = b.materialize(definition);
        b.pulls[self.id] = Some(pull);
    }
}
