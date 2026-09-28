//! Design `baseline`: RFD 3 as written, the control.
//!
//! A token is an index, a generation and a graph id: `Copy` and `'static`,
//! so any `move` closure can capture one and the compiler can't see it.
//! Every capture is declared at run time with `depends`, and a forgotten one
//! is a stale token at use. Types and signatures only, as deep as the
//! fixtures need to type-check; nothing here runs.
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

/// A foreign value that promises to hold no tokens.
pub struct Leaf<T>(pub T);

impl<T> Trace for Leaf<T> {
    fn trace(&self, _: &mut Tracer) {}
}

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

    /// `node` keeps the tokens in `on` alive, whatever a closure does with
    /// them. Checked at run time, if at all.
    pub fn depends(&mut self, node: &impl Trace, on: &[&dyn Trace]) {
        let mut tracer = Tracer { found: Vec::new() };
        node.trace(&mut tracer);
        on.iter().for_each(|value| value.trace(&mut tracer));
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

    fn map<B: 'static, F: Fn(Self::Event) -> B + 'static>(self, _: F) -> Stream<B> {
        self.chain()
    }

    fn filter_map<B: 'static, F: Fn(Self::Event) -> Option<B> + 'static>(self, _: F) -> Stream<B> {
        self.chain()
    }

    /// Traces its value (spike F94), so a token in it needs no declaration.
    fn map_to<B: Trace + Clone + 'static>(self, _: B) -> Stream<B> {
        self.chain()
    }

    fn snapshot<C: 'static, B: 'static, F: Fn(Self::Event, &C) -> B + 'static>(
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

    fn accumulate<S: Trace + 'static, F: Fn(Self::Event, &S) -> S + 'static>(
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

    fn construct<B: 'static, F: Fn(&mut Build, Self::Event) -> B + 'static>(
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
    pub fn map_cell<B: 'static, F: Fn(&A) -> B + 'static>(self, b: &mut Build, _: F) -> Cell<B> {
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
