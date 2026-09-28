//! Design `brand`: every token carries an invariant, generative lifetime
//! `'g`, as gc-arena brands its `Gc<'gc, T>` pointers.
//!
//! Graph code runs inside `Runtime::mutate(|b: &mut Build<'g>| …)`, where
//! `'g` is fresh for each call. Graph closures stay `'static`, so a closure
//! that captures a branded token, or any value holding one, doesn't compile:
//! that is F62 as a type error. Captures travel as data instead, through one
//! adapter, `with(env)`, which pairs each event with a traced environment.
//! Tokens stay usable as data: a cell holds them, a closure receives and
//! returns them. What leaves `mutate` is an `Anchored` handle with the brand
//! erased, which I/O code reopens inside the next `mutate` (gc-arena's
//! `DynamicRoot`).
//!
//! Types and signatures only. A real engine stores closures and values with
//! the brand erased and hands them back at the brand of the current
//! `mutate`, as gc-arena does with `unsafe`; that is sound here for the same
//! reason it is there: a `'static` closure can hold nothing branded, so
//! nothing it keeps can outlive its brand. Where that rebranding would be,
//! the toy has `todo!()`.
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

/// The type a value has under another brand: gc-arena's `Rootable`. Needed
/// only by what crosses `mutate`'s edge, `anchor` and `open`; a derive
/// would write it next to `Trace`.
pub trait Rebrand {
    type Of<'x>;
}

impl<T: 'static> Rebrand for Anchored<T> {
    type Of<'x> = Anchored<T>;
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

/// A root with the brand erased, `'static`, so I/O code can keep it across
/// units. It doesn't deref: the value inside is reached by reopening it in
/// a `mutate`, at that call's brand.
pub struct Anchored<T: 'static> {
    roots: Vec<u32>,
    value: PhantomData<T>,
}

impl<T: 'static> Clone for Anchored<T> {
    fn clone(&self) -> Self {
        Anchored {
            roots: self.roots.clone(),
            value: PhantomData,
        }
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

    pub fn anchor<T: Trace + Rebrand>(&mut self, value: T) -> Anchored<T::Of<'static>>
    where
        T::Of<'static>: 'static,
    {
        let mut tracer = Tracer { found: Vec::new() };
        value.trace(&mut tracer);
        Anchored {
            roots: tracer.found,
            value: PhantomData,
        }
    }

    /// The anchored value at this `mutate`'s brand.
    pub fn open<T: Rebrand + 'static>(&mut self, _: &Anchored<T>) -> T::Of<'g> {
        todo!("rebrand the stored value, unsafe in a real engine as in gc-arena")
    }

    pub fn send<A>(&mut self, _: Input<'g, A>, _: A) {}

    pub fn listen_cell<A>(&mut self, _: Cell<'g, A>, _: impl FnMut(&A) + 'static) -> Listener {
        Listener
    }

    pub fn listen<A>(&mut self, _: Shared<'g, A>, _: impl FnMut(A) + 'static) -> Listener {
        Listener
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

    /// The one way a closure gets a token it didn't receive: the
    /// environment travels with each event, as data the chain traces, the
    /// way `map_to`'s value is traced (spike F93, F94).
    fn with<E: Trace + Clone>(self, _: E) -> Stream<'g, (E, Self::Event)> {
        self.chain()
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

    /// The closure receives this `mutate`'s `Build` when it runs; being
    /// `'static`, it can't have kept anything from the build that made it.
    fn construct<B, F: Fn(&mut Build<'g>, Self::Event) -> B + 'static>(
        self,
        b: &mut Build<'g>,
        _: F,
    ) -> Stream<'g, B> {
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

    /// Build and I/O both go through here. `'g` is fresh for each call, so
    /// nothing branded leaves it: what must, leaves anchored.
    pub fn mutate<R: 'static>(&mut self, f: impl for<'g> FnOnce(&mut Build<'g>) -> R) -> R {
        let mut b = Build {
            next: self.next,
            brand: PhantomData,
        };
        let r = f(&mut b);
        self.next = b.next;
        r
    }

    /// A send needs no brand: the anchored input is enough.
    pub fn send<A: 'static>(&mut self, _: &Anchored<Input<'static, A>>, _: A) {}
}
