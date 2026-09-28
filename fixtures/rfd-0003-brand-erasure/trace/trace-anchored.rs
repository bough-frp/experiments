//! Must fail: a derived `Trace` on a type that holds an `Anchored`. The api
//! leaves `Anchored` without `Trace` so that graph state can't hold a root
//! (the main run's leak route); the derive keeps that, and a skip can't get
//! around it, since `Anchored` implements `Rebrand`.
#![forbid(unsafe_code)]
use bough::*;

#[derive(Clone, Rebrand, Trace)]
struct Holder {
    input: Anchored<Input<'static, u32>>,
}

fn main() {}
