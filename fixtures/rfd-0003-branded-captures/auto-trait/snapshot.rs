//! Must build: a `snapshot` of a cell, which the chain traces.
#![feature(auto_traits, negative_impls)]
#[path = "api.rs"]
mod bough;
use bough::*;

pub fn program() {
    let mut rt = Runtime::new();
    let _ = rt.build(|b| {
        let (n, n_in) = b.input::<u32>();
        let (k, k_in) = b.input::<u32>();
        let latest = k.hold(b, 1);
        let scaled = n.snapshot(latest, |v, k| v * k).hold(b, 0);
        (n_in, k_in, scaled)
    });
}
