//! Illegal: a gate drops the mark of the cell it reads, not of the stream
//! it filters. `c = hold 0 (gate (map (+1) (steps c)) enabled)` is a
//! same-instant cycle through the steps view.
//@ legal no
//@ designs baseline marker
#[cfg_attr(design = "marker", path = "api/marker.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let enabled = b.input::<bool>().hold(&mut b, true);
    let (c, c_loop) = b.cell_loop::<u32>();
    let next = c.steps().map(|n| n + 1).gate(enabled).hold(&mut b, 0);
    c_loop.close(&mut b, next);
}
