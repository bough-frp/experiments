//! Illegal: F3, `c = hold 0 (merge ticks (map (+1) (steps c)))`, with a
//! `depends` declaration naming the forward on the merge. Declaring the
//! reach changes nothing about the cycle.
//@ legal no
//@ designs baseline marker
#[cfg_attr(design = "marker", path = "api/marker.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<u32>();
    let (c, c_loop) = b.cell_loop::<u32>();
    let merged = ticks.merge(&mut b, c.steps().map(|n| n + 1));
    b.depends(&merged, &[&c]);
    let next = merged.hold(&mut b, 0);
    c_loop.close(&mut b, next);
}
