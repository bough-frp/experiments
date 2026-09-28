//! Must fail: a derived struct holding another crate's type with neither
//! `Leaf` nor `#[rebrand(skip)]`.
#![forbid(unsafe_code)]
use bough::*;
use foreign::Celsius;

#[derive(Rebrand)]
struct Reading<'g> {
    sensor: Cell<'g, u32>,
    temperature: Celsius,
}

fn main() {}
