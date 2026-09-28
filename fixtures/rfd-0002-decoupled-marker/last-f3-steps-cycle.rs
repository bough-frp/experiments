//! Illegal: F3 in the marker-last design. The forward is of the epoch
//! the loop closes in, so the close refuses it.
//@ legal no
//@ designs baseline marker-last
#[cfg_attr(design = "marker-last", path = "api/marker-last.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline-last.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<u32>();
    let (mut b, c, c_loop) = b.cell_loop::<u32>();
    let bumped = c.steps().map(|n| n + 1);
    let next = ticks.merge(&mut b, bumped).hold(&mut b, 0);
    let (_b, _c) = c_loop.close(b, next);
}
