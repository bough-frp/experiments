//! Must fail: a type with no lifetime parameter that holds a token at
//! `'static`. The derive takes it for brand-free and writes `Of<'x> = Self`
//! and a view; the field's own `Of` renames the brand, so it doesn't fit.
#![forbid(unsafe_code)]
use bough::*;

#[derive(Rebrand)]
struct Pinned {
    count: Cell<'static, u32>,
}

fn main() {}
