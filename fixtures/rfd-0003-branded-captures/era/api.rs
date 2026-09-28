//! Design `era`: an invariant brand only on the tokens a `construct` mints,
//! ST and GhostCell style, after Jeltsch's era types (synthesis 04).
//!
//! The top-level build is era `'static`, so its tokens are the baseline's:
//! `Copy`, `'static`, capturable. Each run of a `construct` closure gets a
//! fresh era `'e`, `for<'e> Fn(&mut Build<'e>, A) -> …`, so what it mints
//! can't leave except through its return value, which the engine moves to
//! the outer era. The closure itself is `'static`, stored in a graph node.
//! An outer token isn't usable inside an era until `import`ed, and the
//! closure can only import what it captured, so captures are still
//! declared with `depends`. Types and signatures only; the rebranding of a
//! construct's result is `todo!()`, unsafe in a real engine.
#![allow(dead_code)]

use std::marker::PhantomData;

/// Invariant in `'g`, so two brands never unify.
type Brand<'g> = PhantomData<fn(&'g ()) -> &'g ()>;

pub struct Tracer {
    pub found: Vec<u32>,
}

pub trait Trace {
    fn trace(&self, tracer: &mut Tracer);
}

impl Trace for u32 {
    fn trace(&self, _: &mut Tracer) {}
}

impl<A: Trace, B: Trace> Trace for (A, B) {
    fn trace(&self, tracer: &mut Tracer) {
        self.0.trace(tracer);
        self.1.trace(tracer);
    }
}

impl<A: Trace, B: Trace, C: Trace> Trace for (A, B, C) {
    fn trace(&self, tracer: &mut Tracer) {
        self.0.trace(tracer);
        self.1.trace(tracer);
        self.2.trace(tracer);
    }
}

impl<T: Trace> Trace for Vec<T> {
    fn trace(&self, tracer: &mut Tracer) {
        self.iter().for_each(|t| t.trace(tracer));
    }
}

pub struct Leaf<T>(pub T);

impl<T> Trace for Leaf<T> {
    fn trace(&self, _: &mut Tracer) {}
}

/// The type a value has in another era: gc-arena's `Rootable`. Needed by
/// a construct's result and by `import`.
pub trait Rebrand {
    type Of<'x>;
}

impl<T: Rebrand> Rebrand for Anchored<T> {
    type Of<'x> = Anchored<T::Of<'x>>;
}

impl Rebrand for u32 {
    type Of<'x> = u32;
}

impl<A: Rebrand, B: Rebrand> Rebrand for (A, B) {
    type Of<'x> = (A::Of<'x>, B::Of<'x>);
}

impl<A: Rebrand, B: Rebrand, C: Rebrand> Rebrand for (A, B, C) {
    type Of<'x> = (A::Of<'x>, B::Of<'x>, C::Of<'x>);
}

#[derive(Clone, Copy)]
struct Id {
    index: u32,
    generation: u32,
    graph: u32,
}

macro_rules! token {
    ($name:ident) => {
        pub struct $name<'g, A> {
            id: Id,
            event: PhantomData<fn() -> A>,
            brand: Brand<'g>,
        }

        impl<'g, A> Clone for $name<'g, A> {
            fn clone(&self) -> Self {
                *self
            }
        }

        impl<'g, A> Copy for $name<'g, A> {}

        impl<'g, A> Trace for $name<'g, A> {
            fn trace(&self, tracer: &mut Tracer) {
                tracer.found.push(self.id.index);
            }
        }

        impl<'g, A: Rebrand> Rebrand for $name<'g, A> {
            type Of<'x> = $name<'x, A::Of<'x>>;
        }

        impl<'g, A> $name<'g, A> {
            fn at(id: Id) -> Self {
                $name {
                    id,
                    event: PhantomData,
                    brand: PhantomData,
                }
            }
        }
    };
}

token!(Cell);
token!(Shared);
token!(Input);

pub struct Stream<'g, A> {
    id: Id,
    event: PhantomData<fn() -> A>,
    brand: Brand<'g>,
}

impl<'g, A> Stream<'g, A> {
    fn at(id: Id) -> Self {
        Stream {
            id,
            event: PhantomData,
            brand: PhantomData,
        }
    }
}

impl<'g, A> Trace for Stream<'g, A> {
    fn trace(&self, tracer: &mut Tracer) {
        tracer.found.push(self.id.index);
    }
}

impl<'g, A: Rebrand> Rebrand for Stream<'g, A> {
    type Of<'x> = Stream<'x, A::Of<'x>>;
}

/// A guard: roots what it holds, reads as the value.
#[derive(Clone)]
pub struct Anchored<T> {
    value: T,
}

impl<T> std::ops::Deref for Anchored<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}

pub struct Listener;

pub struct Build<'g> {
    next: u32,
    brand: Brand<'g>,
}

impl<'g> Build<'g> {
    fn mint(&mut self) -> Id {
        self.next += 1;
        Id {
            index: self.next,
            generation: 0,
            graph: 0,
        }
    }

    pub fn input<A>(&mut self) -> (Stream<'g, A>, Input<'g, A>) {
        let id = self.mint();
        (Stream::at(id), Input::at(id))
    }

    pub fn stream_loop<A>(&mut self) -> (Stream<'g, A>, StreamLoop<'g, A>) {
        let id = self.mint();
        (
            Stream::at(id),
            StreamLoop {
                stream: Stream::at(id),
            },
        )
    }

    pub fn anchor<T: Trace>(&mut self, value: T) -> Anchored<T> {
        Anchored { value }
    }

    /// An outer token, usable in this era. Only the top-level era's tokens
    /// are `'static`, so only they can be captured and imported.
    pub fn import<T: Rebrand + 'static>(&mut self, _: T) -> T::Of<'g> {
        todo!("rebrand, unsafe in a real engine")
    }

    pub fn depends(&mut self, node: &impl Trace, on: &[&dyn Trace]) {
        let mut tracer = Tracer { found: Vec::new() };
        node.trace(&mut tracer);
        on.iter().for_each(|value| value.trace(&mut tracer));
    }
}

pub struct StreamLoop<'g, A> {
    stream: Stream<'g, A>,
}

impl<'g, A> StreamLoop<'g, A> {
    pub fn close(self, _: &mut Build<'g>, _: impl Source<'g, Event = A>) {}
}

/// Every closure an adapter or materializer stores is `'static`, as in RFD
/// 3; the brand is what makes that bound refuse a captured token.
pub trait Source<'g>: Sized {
    type Event;

    fn id(&self) -> Id;

    fn chain<B>(self) -> Stream<'g, B> {
        Stream::at(self.id())
    }

    fn map<B, F: Fn(Self::Event) -> B + 'static>(self, _: F) -> Stream<'g, B> {
        self.chain()
    }

    fn filter_map<B, F: Fn(Self::Event) -> Option<B> + 'static>(self, _: F) -> Stream<'g, B> {
        self.chain()
    }

    fn map_to<B: Trace + Clone>(self, _: B) -> Stream<'g, B> {
        self.chain()
    }

    fn snapshot<C, B, F: Fn(Self::Event, &C) -> B + 'static>(
        self,
        _: Cell<'g, C>,
        _: F,
    ) -> Stream<'g, B> {
        self.chain()
    }

    fn node(self, b: &mut Build<'g>) -> Stream<'g, Self::Event> {
        Stream::at(b.mint())
    }

    fn share(self, b: &mut Build<'g>) -> Shared<'g, Self::Event>
    where
        Self::Event: Clone,
    {
        Shared::at(b.mint())
    }

    fn hold(self, b: &mut Build<'g>, _: Self::Event) -> Cell<'g, Self::Event>
    where
        Self::Event: Trace,
    {
        Cell::at(b.mint())
    }

    fn accumulate<S: Trace, F: Fn(Self::Event, &S) -> S + 'static>(
        self,
        b: &mut Build<'g>,
        _: S,
        _: F,
    ) -> Cell<'g, S> {
        Cell::at(b.mint())
    }

    /// Each run is a fresh era `'e`; the result `R` is named once for all
    /// eras through `Rebrand`, and moved to this era.
    fn construct<R: Rebrand, F>(self, b: &mut Build<'g>, _: F) -> Stream<'g, R::Of<'g>>
    where
        F: for<'e> Fn(&mut Build<'e>, Self::Event) -> R::Of<'e> + 'static,
    {
        Stream::at(b.mint())
    }
}

impl<'g, A> Source<'g> for Stream<'g, A> {
    type Event = A;
    fn id(&self) -> Id {
        self.id
    }
}

impl<'g, A> Source<'g> for Shared<'g, A> {
    type Event = A;
    fn id(&self) -> Id {
        self.id
    }
}

impl<'g, A, B> Stream<'g, (A, B)> {
    pub fn unzip(self, b: &mut Build<'g>) -> (Stream<'g, A>, Stream<'g, B>) {
        (Stream::at(b.mint()), Stream::at(b.mint()))
    }
}

impl<'g, A> Cell<'g, A> {
    pub fn map_cell<B, F: Fn(&A) -> B + 'static>(self, b: &mut Build<'g>, _: F) -> Cell<'g, B> {
        Cell::at(b.mint())
    }
}

impl<'g, A> Cell<'g, Stream<'g, A>> {
    pub fn switch_stream(self, b: &mut Build<'g>) -> Stream<'g, A> {
        Stream::at(b.mint())
    }
}

impl<'g, A> Cell<'g, Cell<'g, A>> {
    pub fn switch_cell(self, b: &mut Build<'g>) -> Cell<'g, A> {
        Cell::at(b.mint())
    }
}

#[derive(Default)]
pub struct Runtime {
    next: u32,
}

impl Runtime {
    pub fn new() -> Self {
        Runtime { next: 0 }
    }

    /// The top-level build is era `'static`: its tokens are the baseline's.
    pub fn build<T: Trace>(&mut self, f: impl FnOnce(&mut Build<'static>) -> T) -> Anchored<T> {
        let mut b = Build {
            next: self.next,
            brand: PhantomData,
        };
        let value = f(&mut b);
        self.next = b.next;
        Anchored { value }
    }

    pub fn send<A>(&mut self, _: Input<'static, A>, _: A) {}

    pub fn listen<A>(&mut self, _: Shared<'static, A>, _: impl FnMut(A) + 'static) -> Listener {
        Listener
    }

    pub fn listen_cell<A>(&mut self, _: Cell<'static, A>, _: impl FnMut(&A) + 'static) -> Listener {
        Listener
    }
}
