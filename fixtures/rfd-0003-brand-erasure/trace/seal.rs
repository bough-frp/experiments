//! A model Bough for question 3 of the `trace` mode: can a trait only the
//! derive implements close the stash route, where a hand-written `view` (or
//! `Borrow`) copies a `'static` token out of the stored value it is lent?
//!
//! Types only, the few the question needs: a token, the witness, `Rebrand`
//! with `view`, and a store that calls `view`, as `../api.rs` has them. What
//! changes is one supertrait on `Rebrand`, the seal, which the derive
//! implements next to it (built with its `seal` or `unsafe-seal` feature):
//!
//! - default: `__private::Sealed`, a safe trait in a hidden module.
//! - `--cfg unsafe_seal`: `__private::Sound`, an `unsafe` trait, which the
//!   derive implements under `#[allow(unsafe_code)]`. Declaring and
//!   implementing one is `unsafe_code`, so this crate denies it rather than
//!   forbidding it, and allows it on those items; `--cfg core_forbid` keeps
//!   the `forbid` Bough's core has, to show what that costs.
#![cfg_attr(any(not(unsafe_seal), core_forbid), forbid(unsafe_code))]
#![cfg_attr(all(unsafe_seal, not(core_forbid)), deny(unsafe_code))]

pub use rfd_0003_rebrand_derive::Rebrand;

use std::any::Any;
use std::marker::PhantomData;
use std::rc::Rc;

type Brand<'g> = PhantomData<fn(&'g ()) -> &'g ()>;

/// Only this crate makes one.
#[derive(Clone, Copy)]
pub struct Witness<'x> {
    brand: Brand<'x>,
}

fn erased() -> Witness<'static> {
    Witness { brand: PhantomData }
}

/// A token: an id and a brand.
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

impl<'g, A> Cell<'g, A> {
    fn at(id: u32) -> Self {
        Cell {
            id,
            event: PhantomData,
            brand: PhantomData,
        }
    }

    pub fn index(&self) -> u32 {
        self.id
    }
}

/// Hidden, not private: the derive's impl is written in the user's crate,
/// so it must be able to name the seal, and so can the user.
#[doc(hidden)]
pub mod __private {
    #[cfg(not(unsafe_seal))]
    pub trait Sealed {}

    /// # Safety
    ///
    /// Implemented by `#[derive(Rebrand)]` only: its `view` lends nothing.
    #[cfg(unsafe_seal)]
    #[allow(unsafe_code)]
    pub unsafe trait Sound {}
}

#[cfg(not(unsafe_seal))]
use __private::Sealed as Seal;
#[cfg(unsafe_seal)]
use __private::Sound as Seal;

/// Bough's own impls of the seal, for its own types.
macro_rules! seal {
    ($(impl$(<$($g:tt),*>)? for $t:ty;)*) => {$(
        #[cfg(not(unsafe_seal))]
        impl$(<$($g),*>)? Seal for $t {}
        #[cfg(unsafe_seal)]
        #[allow(unsafe_code)]
        unsafe impl$(<$($g),*>)? Seal for $t {}
    )*};
}

pub trait Rebrand: Sized + Seal {
    type Of<'x>: 'x;
    fn rebrand<'x>(&self, to: Witness<'x>) -> Self::Of<'x>;
    fn restore<'x>(from: &Self::Of<'x>, at: Witness<'x>) -> Self;
    fn view<'a>(from: &'a Self::Of<'static>) -> Option<&'a Self> {
        let _ = from;
        None
    }
}

seal! {
    impl for u32;
    impl for String;
    impl<'g, A> for Cell<'g, A>;
}

macro_rules! plain {
    ($($t:ty),*) => {$(
        impl Rebrand for $t {
            type Of<'x> = $t;
            fn rebrand<'x>(&self, _: Witness<'x>) -> $t {
                self.clone()
            }
            fn restore<'x>(from: &$t, _: Witness<'x>) -> $t {
                from.clone()
            }
            fn view(from: &$t) -> Option<&$t> {
                Some(from)
            }
        }
    )*};
}

plain!(u32, String);

impl<'g, A: Rebrand> Rebrand for Cell<'g, A> {
    type Of<'x> = Cell<'x, A::Of<'x>>;
    fn rebrand<'x>(&self, _: Witness<'x>) -> Self::Of<'x> {
        Cell::at(self.id)
    }
    fn restore<'x>(from: &Self::Of<'x>, _: Witness<'x>) -> Self {
        Cell::at(from.id)
    }
}

/// A `mutate`: tokens minted at its brand, values stored at `'static` and
/// read back through `view` or a restore, as the api's engine does.
pub struct Scope<'g> {
    next: u32,
    brand: Brand<'g>,
}

pub fn mutate<R>(f: impl for<'g> FnOnce(&mut Scope<'g>) -> R) -> R {
    f(&mut Scope {
        next: 0,
        brand: PhantomData,
    })
}

/// A stored value, at brand `'static`.
pub struct Stored(Rc<dyn Any>);

impl<'g> Scope<'g> {
    pub fn cell(&mut self) -> Cell<'g, u32> {
        self.next += 1;
        Cell::at(self.next)
    }

    pub fn store<T: Rebrand>(&self, value: &T) -> Stored {
        Stored(Rc::new(value.rebrand(erased())))
    }

    pub fn read<T: Rebrand, R>(&self, stored: &Stored, f: impl FnOnce(&T) -> R) -> R {
        let stored = stored
            .0
            .downcast_ref::<T::Of<'static>>()
            .expect("the type it was stored at");
        match T::view(stored) {
            Some(value) => f(value),
            None => f(&T::restore(stored, erased())),
        }
    }
}
