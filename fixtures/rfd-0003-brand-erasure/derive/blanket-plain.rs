//! Must build, against `blanket.rs`: a foreign type is `Rebrand` through
//! the blanket impl, with no wrapper, and a derive on a local type that
//! isn't `Clone` doesn't overlap it: coherence may rely on a local type
//! not being `Clone`, since only this crate could make it so.
#![forbid(unsafe_code)]
use bough::*;
use foreign::Celsius;

#[derive(Rebrand)]
struct Reading {
    temperature: Celsius,
    count: u32,
}

fn stored<T: Rebrand>() {}

fn main() {
    stored::<Celsius>();
    stored::<Reading>();
}
