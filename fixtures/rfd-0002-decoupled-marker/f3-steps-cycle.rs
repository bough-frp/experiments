//! F3: `c = hold 0 (merge ticks (map (+1) (steps c)))`. The path back to
//! `c` passes through a hold, but through its steps view, which the hold
//! does not delay: a same-instant cycle.
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
    let bumped = c.steps().map(|n| n + 1);
    let next = ticks.merge(&mut b, bumped).hold(&mut b, 0);
    c_loop.close(&mut b, next);
}
