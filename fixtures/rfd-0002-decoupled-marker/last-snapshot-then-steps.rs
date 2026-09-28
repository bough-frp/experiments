//! `close-snapshot-then-steps` in the marker-last design. Legal.
//@ legal yes
//@ designs baseline marker-last
#[cfg_attr(design = "marker-last", path = "api/marker-last.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline-last.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<()>();
    let (b, x, x_loop) = b.cell_loop::<u32>();
    let (mut b, y, y_loop) = b.cell_loop::<u32>();
    let next_x = ticks.snapshot(y, |_, n| n + 1).hold(&mut b, 0);
    let (mut b, x) = x_loop.close(b, next_x);
    let next_y = x.steps().snapshot(y, |a, n| a + n).hold(&mut b, 0);
    let (_b, _y) = y_loop.close(b, next_y);
}
