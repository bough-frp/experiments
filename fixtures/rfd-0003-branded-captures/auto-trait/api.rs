//! Design `auto-trait`: the baseline's tokens, plus a nightly auto trait,
//! `Stable`, that every token type opts out of.
//!
//! An auto trait propagates through fields and through a closure's
//! captures, the way `Send` does, so a closure that captures a token, or a
//! struct, `Vec` or `Rc` holding one, is not `Stable`. Every graph closure
//! is bounded `Stable + 'static`: F62 is a type error, with no lifetime
//! anywhere. Captures travel as data through `with(env)`, as in `brand`.
//! This is the Rust analogue of the modal line's stable types (synthesis
//! 02). Needs `#![feature(auto_traits, negative_impls)]` in the crate root,
//! so each fixture carries it. Types and signatures only.
#![allow(dead_code)]

use std::marker::PhantomData;
use std::ops::Deref;

/// What a trace walk collects: the arena index of every token a value names.
pub struct Tracer {
    pub found: Vec<u32>,
}

/// Every value the graph persists implements `Trace`, derived in the real
/// crate; the fixtures write their impls by hand.
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

/// A foreign value that promises to hold no tokens. It promises the same to
/// `Stable`, which lets a trait object through.
pub struct Leaf<T>(pub T);

impl<T> Trace for Leaf<T> {
    fn trace(&self, _: &mut Tracer) {}
}

impl<T> Stable for Leaf<T> {}

/// Holds no token, so a graph closure may capture it. Auto: implemented for
/// every type whose fields all implement it, and for a closure whose
/// captures all do.
#[diagnostic::on_unimplemented(
    message = "`{Self}` may hold a token, so a graph closure can't capture it",
    label = "captured by this closure",
    note = "pass it with `.with(..)` so the node traces it, or wrap a value that holds no token in `Leaf`"
)]
pub auto trait Stable {}

#[derive(Clone, Copy)]
struct Id {
    index: u32,
    generation: u32,
    graph: u32,
}

macro_rules! token {
    ($name:ident) => {
        pub struct $name<A> {
            id: Id,
            event: PhantomData<fn() -> A>,
        }

        impl<A> Clone for $name<A> {
            fn clone(&self) -> Self {
                *self
            }
        }

        impl<A> Copy for $name<A> {}

        impl<A> Trace for $name<A> {
            fn trace(&self, tracer: &mut Tracer) {
                tracer.found.push(self.id.index);
            }
        }
    };
}

token!(Cell);
token!(Shared);
token!(Input);

impl<A> !Stable for Cell<A> {}
impl<A> !Stable for Shared<A> {}
impl<A> !Stable for Input<A> {}
impl<A> !Stable for Stream<A> {}

/// Move-only: a stream has at most one consumer (RFD 4).
pub struct Stream<A> {
    id: Id,
    event: PhantomData<fn() -> A>,
}

impl<A> Trace for Stream<A> {
    fn trace(&self, tracer: &mut Tracer) {
        tracer.found.push(self.id.index);
    }
}

/// A guard: roots what it holds, reads as the value. Its clones share one
/// root. Not `Trace`, so a hold of one doesn't compile.
#[derive(Clone)]
pub struct Anchored<T> {
    value: T,
}

impl<T> Deref for Anchored<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}

pub struct Listener;

pub struct Build {
    next: u32,
}

impl Build {
    fn mint(&mut self) -> Id {
        self.next += 1;
        Id {
            index: self.next,
            generation: 0,
            graph: 0,
        }
    }

    pub fn input<A>(&mut self) -> (Stream<A>, Input<A>) {
        let id = self.mint();
        let event = PhantomData;
        (Stream { id, event }, Input { id, event })
    }

    pub fn stream_loop<A>(&mut self) -> (Stream<A>, StreamLoop<A>) {
        let id = self.mint();
        (
            Stream {
                id,
                event: PhantomData,
            },
            StreamLoop {
                id,
                event: PhantomData,
            },
        )
    }

    pub fn anchor<T: Trace>(&mut self, value: T) -> Anchored<T> {
        Anchored { value }
    }
}

pub struct StreamLoop<A> {
    id: Id,
    event: PhantomData<fn() -> A>,
}

impl<A: 'static> StreamLoop<A> {
    pub fn close(self, _: &mut Build, _: impl Source<Event = A>) {}
}

/// `Stream` and `Shared`; the adapters return a `Stream` here, where the
/// real crate returns its own adapter type per adapter.
pub trait Source: Sized {
    type Event: 'static;

    fn id(&self) -> Id;

    fn chain<B>(self) -> Stream<B> {
        Stream {
            id: self.id(),
            event: PhantomData,
        }
    }

    /// The one way a closure gets a token it didn't receive: the
    /// environment travels with each event, as data the chain traces.
    fn with<E: Trace + Clone + 'static>(self, _: E) -> Stream<(E, Self::Event)> {
        self.chain()
    }

    fn map<B: 'static, F: Fn(Self::Event) -> B + Stable + 'static>(self, _: F) -> Stream<B> {
        self.chain()
    }

    fn filter_map<B: 'static, F: Fn(Self::Event) -> Option<B> + Stable + 'static>(
        self,
        _: F,
    ) -> Stream<B> {
        self.chain()
    }

    /// Traces its value (spike F94), so a token in it needs no declaration.
    fn map_to<B: Trace + Clone + 'static>(self, _: B) -> Stream<B> {
        self.chain()
    }

    fn snapshot<C: 'static, B: 'static, F: Fn(Self::Event, &C) -> B + Stable + 'static>(
        self,
        _: Cell<C>,
        _: F,
    ) -> Stream<B> {
        self.chain()
    }

    fn node(self, b: &mut Build) -> Stream<Self::Event> {
        Stream {
            id: b.mint(),
            event: PhantomData,
        }
    }

    fn share(self, b: &mut Build) -> Shared<Self::Event>
    where
        Self::Event: Clone,
    {
        Shared {
            id: b.mint(),
            event: PhantomData,
        }
    }

    fn hold(self, b: &mut Build, _: Self::Event) -> Cell<Self::Event>
    where
        Self::Event: Trace,
    {
        Cell {
            id: b.mint(),
            event: PhantomData,
        }
    }

    fn accumulate<S: Trace + 'static, F: Fn(Self::Event, &S) -> S + Stable + 'static>(
        self,
        b: &mut Build,
        _: S,
        _: F,
    ) -> Cell<S> {
        Cell {
            id: b.mint(),
            event: PhantomData,
        }
    }

    fn construct<B: 'static, F: Fn(&mut Build, Self::Event) -> B + Stable + 'static>(
        self,
        b: &mut Build,
        _: F,
    ) -> Stream<B> {
        self.node(b).chain()
    }
}

impl<A: 'static> Source for Stream<A> {
    type Event = A;
    fn id(&self) -> Id {
        self.id
    }
}

impl<A: 'static> Source for Shared<A> {
    type Event = A;
    fn id(&self) -> Id {
        self.id
    }
}

impl<A: 'static, B: 'static> Stream<(A, B)> {
    pub fn unzip(self, b: &mut Build) -> (Stream<A>, Stream<B>) {
        let right = b.mint();
        let left = self.node(b).id;
        (
            Stream {
                id: left,
                event: PhantomData,
            },
            Stream {
                id: right,
                event: PhantomData,
            },
        )
    }
}

impl<A: 'static> Cell<A> {
    pub fn map_cell<B: 'static, F: Fn(&A) -> B + Stable + 'static>(
        self,
        b: &mut Build,
        _: F,
    ) -> Cell<B> {
        Cell {
            id: b.mint(),
            event: PhantomData,
        }
    }
}

impl<A> Cell<Stream<A>> {
    pub fn switch_stream(self, b: &mut Build) -> Stream<A> {
        Stream {
            id: b.mint(),
            event: PhantomData,
        }
    }
}

impl<A> Cell<Cell<A>> {
    pub fn switch_cell(self, b: &mut Build) -> Cell<A> {
        Cell {
            id: b.mint(),
            event: PhantomData,
        }
    }
}

#[derive(Default)]
pub struct Runtime {
    build: Option<Build>,
}

impl Runtime {
    pub fn new() -> Self {
        Runtime {
            build: Some(Build { next: 0 }),
        }
    }

    /// The build's return comes back anchored (RFD 3).
    pub fn build<T: Trace>(&mut self, f: impl FnOnce(&mut Build) -> T) -> Anchored<T> {
        let b = self.build.as_mut().expect("a runtime");
        Anchored { value: f(b) }
    }

    pub fn send<A>(&mut self, _: Input<A>, _: A) {}

    pub fn listen<A: 'static>(&mut self, _: Shared<A>, _: impl FnMut(A) + 'static) -> Listener {
        Listener
    }

    pub fn listen_cell<A: 'static>(&mut self, _: Cell<A>, _: impl FnMut(&A) + 'static) -> Listener {
        Listener
    }
}
