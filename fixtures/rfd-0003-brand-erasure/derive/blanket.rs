//! A Bough that answers the orphan rules with a blanket impl: every
//! `Clone + 'static` type is its own `Of`, so a third crate's type needs no
//! wrapper. Types only, as crate `bough`, like `bough.rs`.
//!
//! Built plain, it has no token. Built `--cfg with_token`, it adds the
//! api's impl for a token, which coherence checks against the blanket one.
#![forbid(unsafe_code)]

use std::marker::PhantomData;

pub use rfd_0003_rebrand_derive::Rebrand;

type Brand<'g> = PhantomData<fn(&'g ()) -> &'g ()>;

#[derive(Clone, Copy)]
pub struct Witness<'x> {
    brand: Brand<'x>,
}

pub trait Rebrand: Sized {
    type Of<'x>: 'x;
    fn rebrand<'x>(&self, to: Witness<'x>) -> Self::Of<'x>;
    fn restore<'x>(from: &Self::Of<'x>, at: Witness<'x>) -> Self;
    fn view<'a>(from: &'a Self::Of<'static>) -> Option<&'a Self> {
        let _ = from;
        None
    }
}

impl<T: Clone + 'static> Rebrand for T {
    type Of<'x> = T;
    fn rebrand<'x>(&self, _: Witness<'x>) -> T {
        self.clone()
    }
    fn restore<'x>(from: &T, _: Witness<'x>) -> T {
        from.clone()
    }
    fn view(from: &T) -> Option<&T> {
        Some(from)
    }
}

/// A token is `Copy`, as every token in the api is.
pub struct Cell<'g, A> {
    id: u32,
    event: PhantomData<fn() -> A>,
    brand: Brand<'g>,
}

impl<'g, A> Clone for Cell<'g, A> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'g, A> Copy for Cell<'g, A> {}

#[cfg(with_token)]
impl<'g, A: Rebrand> Rebrand for Cell<'g, A> {
    type Of<'x> = Cell<'x, A::Of<'x>>;
    fn rebrand<'x>(&self, _: Witness<'x>) -> Self::Of<'x> {
        Cell {
            id: self.id,
            event: PhantomData,
            brand: PhantomData,
        }
    }
    fn restore<'x>(from: &Self::Of<'x>, _: Witness<'x>) -> Self {
        Cell {
            id: from.id,
            event: PhantomData,
            brand: PhantomData,
        }
    }
}
