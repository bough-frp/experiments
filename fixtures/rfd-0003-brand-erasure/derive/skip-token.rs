//! Must fail: `#[rebrand(skip)]` on a field that holds the brand. A skipped
//! field is cloned as it is, so it would carry brand `'g` into `Of<'x>`;
//! its type is the same in both, and the brands don't unify.
#![forbid(unsafe_code)]
use bough::*;

#[derive(Rebrand)]
struct Sneaky<'g> {
    #[rebrand(skip)]
    count: Cell<'g, u32>,
}

fn main() {}
