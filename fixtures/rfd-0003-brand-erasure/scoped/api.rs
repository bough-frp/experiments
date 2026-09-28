//! Design `scoped` of `rfd-0003-brand-erasure`, types and signatures only:
//! the default design's brand, plus a second brand `'r` for the runtime,
//! generative, from `runtime(|rt| ...)`. `Anchored<'r, T>` carries it, so it
//! isn't `'static`, and a graph closure, which is, can't capture one: the
//! leak route of question 3 as a type error. I/O code holds an anchored
//! value only inside the runtime's scope, so a thread that must be
//! `'static` (`std::thread::spawn`, `tokio::spawn`) can't hold one; a scoped
//! thread can.
#![forbid(unsafe_code)]
#![allow(dead_code)]

use std::marker::PhantomData;

type Brand<'a> = PhantomData<fn(&'a ()) -> &'a ()>;

pub trait Rebrand {
    type Of<'x>: 'x;
}

impl Rebrand for u32 {
    type Of<'x> = u32;
}

impl<A: Rebrand, B: Rebrand> Rebrand for (A, B) {
    type Of<'x> = (A::Of<'x>, B::Of<'x>);
}

macro_rules! token {
    ($name:ident) => {
        pub struct $name<'g, A> {
            index: u32,
            event: PhantomData<fn() -> A>,
            brand: Brand<'g>,
        }

        impl<'g, A> Clone for $name<'g, A> {
            fn clone(&self) -> Self {
                *self
            }
        }

        impl<'g, A> Copy for $name<'g, A> {}

        impl<'g, A: Rebrand> Rebrand for $name<'g, A> {
            type Of<'x> = $name<'x, A::Of<'x>>;
        }

        impl<'g, A> $name<'g, A> {
            fn at(index: u32) -> Self {
                $name {
                    index,
                    event: PhantomData,
                    brand: PhantomData,
                }
            }
        }
    };
}

token!(Cell);
token!(Stream);
token!(Input);

/// Not `'static`: it names the runtime it came from.
pub struct Anchored<'r, T> {
    value: PhantomData<T>,
    runtime: Brand<'r>,
}

impl<'r, T> Clone for Anchored<'r, T> {
    fn clone(&self) -> Self {
        Anchored {
            value: PhantomData,
            runtime: PhantomData,
        }
    }
}

pub struct Runtime<'r> {
    runtime: Brand<'r>,
}

/// The runtime lives for the closure; `'r` is fresh for each call.
pub fn runtime<R>(f: impl for<'r> FnOnce(Runtime<'r>) -> R) -> R {
    f(Runtime {
        runtime: PhantomData,
    })
}

pub struct Build<'r, 'g> {
    runtime: Brand<'r>,
    brand: Brand<'g>,
}

impl<'r> Runtime<'r> {
    /// No `'static` bound on `R`: an `Anchored<'r, _>` may leave, a token
    /// branded `'g` can't, since `R` is chosen outside the `for<'g>`.
    pub fn mutate<R>(&mut self, f: impl for<'g> FnOnce(&mut Build<'r, 'g>) -> R) -> R {
        f(&mut Build {
            runtime: PhantomData,
            brand: PhantomData,
        })
    }

    pub fn remote_io(&self) -> RemoteIo {
        RemoteIo
    }

    pub fn pump(&mut self) {}
}

impl<'r, 'g> Build<'r, 'g> {
    pub fn input<A>(&mut self) -> (Stream<'g, A>, Input<'g, A>) {
        (Stream::at(0), Input::at(0))
    }

    pub fn anchor<T: Rebrand>(&mut self, _: T) -> Anchored<'r, T::Of<'static>> {
        Anchored {
            value: PhantomData,
            runtime: PhantomData,
        }
    }

    pub fn open<T: Rebrand>(&self, _: &Anchored<'r, T>) -> T::Of<'g> {
        todo!("as the default design's open")
    }

    pub fn send<A>(&mut self, _: Input<'g, A>, _: A) {}
}

impl<'g, A> Stream<'g, A> {
    pub fn hold<'r>(self, _: &mut Build<'r, 'g>, _: A) -> Cell<'g, A> {
        Cell::at(0)
    }

    pub fn construct<'r, B, F: Fn(&mut Build<'r, 'g>, A) -> B + 'static>(
        self,
        _: &mut Build<'r, 'g>,
        _: F,
    ) -> Stream<'g, B> {
        Stream::at(0)
    }
}

/// `Send + Sync + Clone + 'static`: only the anchors carry the runtime's
/// brand, so a failure to spawn is theirs.
#[derive(Clone)]
pub struct RemoteIo;

impl RemoteIo {
    pub fn send<A: Send + 'static>(&self, _: &Anchored<'_, Input<'static, A>>, _: A) {}
}
