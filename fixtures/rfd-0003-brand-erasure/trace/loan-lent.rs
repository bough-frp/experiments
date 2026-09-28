//! Must fail: the Loan-style answer, the stored copy lent at the loan's own
//! brand, so that a token copied out of it keeps a brand local to the call.
//! Standalone: the engine stores `T::Of<'static>` and would lend
//! `&'a T::Of<'a>`. The brand is invariant (two brands must never unify),
//! so `Of<'static>` is not an `Of<'a>`, generically or for a token; only a
//! cast would convert it. A copy at a local brand is `restore`, which is the
//! copy the view exists to avoid.
#![forbid(unsafe_code)]
use std::marker::PhantomData;

type Brand<'g> = PhantomData<fn(&'g ()) -> &'g ()>;

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

fn lend<'a, T: Rebrand>(stored: &'a T::Of<'static>) -> &'a T::Of<'a> {
    stored
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
    let _ = (lent.id, lend::<Cell<'_>>(&stored).id);
}
