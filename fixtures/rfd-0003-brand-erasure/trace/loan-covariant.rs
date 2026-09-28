//! Builds, and is no answer: `loan-lent.rs` with a covariant brand, the one
//! change that lets `&'a Cell<'static>` be lent as `&'a Cell<'a>`. The
//! lending works for a token (not generically: `T::Of<'static>` and
//! `T::Of<'a>` are still two projections), and a covariant brand lets any
//! token shrink to any brand, so two `mutate`s' tokens unify and a stashed
//! `'static` token is usable everywhere without even `anchor`.
#![forbid(unsafe_code)]
use std::marker::PhantomData;

type Brand<'g> = PhantomData<&'g ()>;

trait Rebrand {
    type Of<'x>: 'x;
}

struct Cell<'g> {
    id: u32,
    brand: Brand<'g>,
}

impl<'g> Rebrand for Cell<'g> {
    type Of<'x> = Cell<'x>;
}

fn lend_cell<'a>(stored: &'a Cell<'static>) -> &'a Cell<'a> {
    stored
}

fn main() {
    let stored = Cell {
        id: 1,
        brand: PhantomData,
    };
    let lent = lend_cell(&stored);
    let _ = lent.id;
}
