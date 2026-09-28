//! Illegal: `close-two-loops-cycle` in the marker-last design. Whichever
//! loop closes first leaves the other open, so the epoch stays and the
//! other's forward is current.
//@ legal no
//@ designs baseline marker-last
#[cfg_attr(design = "marker-last", path = "api/marker-last.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline-last.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<u32>();
    let (b, x, x_loop) = b.cell_loop::<u32>();
    let (mut b, y, y_loop) = b.cell_loop::<u32>();
    let _ = x;
    let next_x = ticks
        .merge(&mut b, y.steps().map(|n| n + 1))
        .hold(&mut b, 0);
    let (mut b, x) = x_loop.close(b, next_x);
    let next_y = x.steps().hold(&mut b, 0);
    let (_b, _y) = y_loop.close(b, next_y);
}
