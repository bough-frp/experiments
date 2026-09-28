//! Illegal: two builds trade a loop to put the count out of step. `w`,
//! opened on `b2`, is closed on `b1`, so `b1` counts no loop open and
//! moves to a new epoch while `x` is still open on it. `y` then reads
//! `x`'s forward as an old one and closes; `x` closes on `b2`, still in the
//! first epoch, from `y`'s returned token. `x` and `y` depend on each
//! other in one instant. RFD 2 panics at the cross-build close.
//@ legal no
//@ designs baseline marker-last
#[cfg_attr(design = "marker-last", path = "api/marker-last.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline-last.rs")]
mod bough;
use bough::*;

pub fn program() {
    let b1 = Build::new();
    let b2 = Build::new();
    let (mut b1, x, x_loop) = b1.cell_loop::<u32>();
    let (b2, _w, w_loop) = b2.cell_loop::<u32>();
    let zero = b1.constant(0);
    let (b1, _w) = w_loop.close(b1, zero);

    let (mut b1, y, y_loop) = b1.cell_loop::<u32>();
    let ticks = b1.input::<u32>();
    let _ = y;
    let next_y = ticks.merge(&mut b1, x.steps()).hold(&mut b1, 0);
    let (mut b1, y) = y_loop.close(b1, next_y);
    let next_x = y.steps().map(|n| n + 1).hold(&mut b1, 0);
    let (_b2, _x) = x_loop.close(b2, next_x);
}
