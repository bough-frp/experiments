//! Illegal: `two-loops-cycle`, trying the returned token. Each loop reads
//! the other's steps; whichever closes first reads the other's forward
//! while that loop is still open, so no returned token exists yet to read
//! instead. Here `x` closes first.
//@ legal no
//@ designs baseline marker-close
#[cfg_attr(design = "marker-close", path = "api/marker-close.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline-close.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<u32>();
    let (x, x_loop) = b.cell_loop::<u32>();
    let (y, y_loop) = b.cell_loop::<u32>();
    let _ = x;
    let next_x = ticks
        .merge(&mut b, y.steps().map(|n| n + 1))
        .hold(&mut b, 0);
    let x = x_loop.close(&mut b, next_x);
    let next_y = x.steps().hold(&mut b, 0);
    let _y = y_loop.close(&mut b, next_y);
}
