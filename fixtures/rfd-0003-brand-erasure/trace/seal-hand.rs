//! Must fail: the stash of `borrow/stash-in-view.rs`, a hand-written
//! `view` that copies the stored `'static` token into a thread-local, under
//! the safe seal. The impl lacks the seal, which only the derive writes.
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

fn main() {}
