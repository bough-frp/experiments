//! Two loops opened together, each reading the other: the first reads the
//! second from before the instant, through `snapshot`, and closes first;
//! the second reads the first's steps through the token the close
//! returned. `a` depends on `b` only across instants, so the graph is
//! acyclic. The returned token is what lets the second loop read the
//! first's steps; the forward would be refused.
//@ legal yes
//@ designs baseline marker-close
#[cfg_attr(design = "marker-close", path = "api/marker-close.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline-close.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<()>();
    let (x, x_loop) = b.cell_loop::<u32>();
    let (y, y_loop) = b.cell_loop::<u32>();
    let next_x = ticks.snapshot(y, |_, n| n + 1).hold(&mut b, 0);
    let x = x_loop.close(&mut b, next_x);
    let next_y = x.steps().snapshot(y, |a, n| a + n).hold(&mut b, 0);
    let _y = y_loop.close(&mut b, next_y);
}
