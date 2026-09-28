//! Must fail: `Trace` derived alone, `Rebrand` by hand, and a `'static`
//! token skipped inside two containers. The check is the `Trace` derive's
//! here: `Option<Vec<Cell<'static, u32>>>` implements `Rebrand`, through the
//! api's impls for each layer.
#![forbid(unsafe_code)]
use bough::*;

#[derive(Clone, Trace)]
struct Pinned {
    #[rebrand(skip)]
    counts: Option<Vec<Cell<'static, u32>>>,
}

impl Rebrand for Pinned {
    type Of<'x> = Pinned;
    fn rebrand<'x>(&self, _: Witness<'x>) -> Pinned {
        self.clone()
    }
    fn restore<'x>(from: &Pinned, _: Witness<'x>) -> Pinned {
        from.clone()
    }
}

fn main() {}
