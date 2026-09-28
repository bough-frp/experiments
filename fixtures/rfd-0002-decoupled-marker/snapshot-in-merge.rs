//! A loop through `snapshot` inside a merge: a counter with a reset,
//! `c = hold 0 (merge (snapshot (+1) ticks c) (map (const 0) resets))`.
//@ legal yes
//@ designs baseline marker
#[cfg_attr(design = "marker", path = "api/marker.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<()>();
    let resets = b.input::<()>();
    let (c, c_loop) = b.cell_loop::<u32>();
    let bumped = ticks.snapshot(c, |_, n| n + 1);
    let next = bumped.merge(&mut b, resets.map(|_| 0)).hold(&mut b, 0);
    c_loop.close(&mut b, next);
}
