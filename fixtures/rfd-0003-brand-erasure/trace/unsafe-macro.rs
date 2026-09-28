//! Builds, and shouldn't: `unsafe-hand-forbid.rs` with the `unsafe impl`
//! written by a `macro_rules!` from another crate, `helper.rs`. The lint
//! isn't reported there either, so under `forbid` the seal is exactly as
//! strong as `forbid(unsafe_code)` itself, which never covered other
//! crates' macros.
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

helper::unsafe_impl!(bough::__private::Sound, Pair<'_>);

fn main() {}
