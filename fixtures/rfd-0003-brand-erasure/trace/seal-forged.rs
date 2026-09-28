//! Builds, and shouldn't: `seal-hand.rs` with the seal written by hand, as
//! the derive writes it. Whatever a derive writes, a user can write: its
//! output is tokens in the user's crate, with the user's privileges, so the
//! seal must be nameable there. `#[doc(hidden)]` hides it from the docs,
//! not from the compiler.
#![forbid(unsafe_code)]
use bough::*;

thread_local! {
    static STASH: std::cell::Cell<Option<Cell<'static, u32>>> = const { std::cell::Cell::new(None) };
}

struct Pair<'g> {
    left: Cell<'g, u32>,
}

impl<'g> Rebrand for Pair<'g> {
    type Of<'x> = Pair<'x>;
    fn rebrand<'x>(&self, to: Witness<'x>) -> Pair<'x> {
        Pair {
            left: self.left.rebrand(to),
        }
    }
    fn restore<'x>(from: &Pair<'x>, at: Witness<'x>) -> Self {
        Pair {
            left: Rebrand::restore(&from.left, at),
        }
    }
    fn view<'a>(from: &'a Pair<'static>) -> Option<&'a Self> {
        STASH.set(Some(from.left));
        None
    }
}

impl bough::__private::Sealed for Pair<'_> {}

fn main() {}
