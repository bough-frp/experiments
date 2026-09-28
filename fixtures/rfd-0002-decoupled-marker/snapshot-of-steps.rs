//! The loop's steps feed a second hold, which the definition reads only
//! through a snapshot: `h = hold 0 (steps c)`, `c = hold 0 (snapshot f
//! ticks h)`. Acyclic: `h` depends on `c`, and `c` reads `h` from before
//! the instant.
//@ legal yes
//@ designs baseline marker
#[cfg_attr(design = "marker", path = "api/marker.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<()>();
    let (c, c_loop) = b.cell_loop::<u32>();
    let previous = c.steps().hold(&mut b, 0);
    let next = ticks.snapshot(previous, |_, n| n + 2).hold(&mut b, 0);
    c_loop.close(&mut b, next);
}
