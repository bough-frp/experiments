//! Illegal: a cell loop through `map_cell` only, `c = map_cell (+1) c`,
//! the `lift(forward, ...)` case RFD 2 names.
//@ legal no
//@ designs baseline marker
#[cfg_attr(design = "marker", path = "api/marker.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let (c, c_loop) = b.cell_loop::<u32>();
    let next = c.map_cell(&mut b, |n| n + 1);
    c_loop.close(&mut b, next);
}
