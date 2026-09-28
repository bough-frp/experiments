//! Must fail, against `blanket.rs`: a derive on a `Clone` type with a brand,
//! as every struct of tokens is (tokens are `Copy`). `Reading<'g>` is
//! `'static` for `'g = 'static`, and coherence doesn't look at lifetimes.
#![forbid(unsafe_code)]
use bough::*;

#[derive(Clone, Rebrand)]
struct Reading<'g> {
    count: Cell<'g, u32>,
}

fn main() {}
