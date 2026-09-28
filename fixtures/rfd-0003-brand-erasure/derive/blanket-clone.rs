//! Must fail, against `blanket.rs`: a derive on a local type that is
//! `Clone`. The blanket impl already covers it, since coherence ignores the
//! `'static` bound, so the derived impl conflicts with Bough's.
#![forbid(unsafe_code)]
use bough::*;
use foreign::Celsius;

#[derive(Clone, Rebrand)]
struct Reading {
    temperature: Celsius,
    count: u32,
}

fn main() {}
