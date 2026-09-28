//! Illegal: two loops, each reading the other's steps.
//! `a = hold 0 (merge ticks (map (+1) (steps b)))`, `b = hold 0 (steps a)`.
//@ legal no
//@ designs baseline marker
#[cfg_attr(design = "marker", path = "api/marker.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<u32>();
    let (x, x_loop) = b.cell_loop::<u32>();
    let (y, y_loop) = b.cell_loop::<u32>();
    let next_x = ticks
        .merge(&mut b, y.steps().map(|n| n + 1))
        .hold(&mut b, 0);
    x_loop.close(&mut b, next_x);
    let next_y = x.steps().hold(&mut b, 0);
    y_loop.close(&mut b, next_y);
}
