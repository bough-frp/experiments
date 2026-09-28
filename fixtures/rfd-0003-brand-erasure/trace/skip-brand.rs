//! Must fail: a skipped field that holds the brand. The committed derive
//! refused it too, as `lifetime may not live long enough` from inside its
//! impl (`derive/skip-token.rs`); the checked one names the field.
#![forbid(unsafe_code)]
use bough::*;

#[derive(Clone, Rebrand, Trace)]
struct Sneaky<'g> {
    #[rebrand(skip)]
    count: Cell<'g, u32>,
}

fn main() {}
