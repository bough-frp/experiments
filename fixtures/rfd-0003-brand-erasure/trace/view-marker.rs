//! Must fail: `view` for a generic type by a marker bound on the method,
//! `T: Rebrand<Of<'static> = T>` (the equality the derive's `Of` meets for a
//! brand-free `T`, a marker trait with no impl to write). `Some(from)` then
//! type-checks, but the trait's `view` has no such bound, and an impl can't
//! add one (E0271, with a note that the requirement is on the impl's `view`
//! and not the trait's). Picking this body when the bound holds and `None` when it
//! doesn't is specialization.
#![forbid(unsafe_code)]
use bough::*;

#[derive(Clone)]
struct Tagged<T> {
    tag: String,
    value: T,
}

impl<T: Rebrand> Rebrand for Tagged<T> {
    type Of<'x> = Tagged<T::Of<'x>>;
    fn rebrand<'x>(&self, to: Witness<'x>) -> Self::Of<'x> {
        Tagged {
            tag: self.tag.clone(),
            value: self.value.rebrand(to),
        }
    }
    fn restore<'x>(from: &Self::Of<'x>, at: Witness<'x>) -> Self {
        Tagged {
            tag: from.tag.clone(),
            value: T::restore(&from.value, at),
        }
    }
    fn view<'a>(from: &'a Self::Of<'static>) -> Option<&'a Self>
    where
        T: Rebrand<Of<'static> = T>,
    {
        Some(from)
    }
}

fn main() {}
