//! Illegal: `close-own-loop-again` in the marker-last design. The first
//! loop's close moves the epoch on; the second loop's forward is of the
//! new one, and its definition reads it.
//@ legal no
//@ designs baseline marker-last
#[cfg_attr(design = "marker-last", path = "api/marker-last.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline-last.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<()>();
    let (mut b, c, c_loop) = b.cell_loop::<u32>();
    let next_c = ticks.snapshot(c, |_, n| n + 1).hold(&mut b, 0);
    let (b, c) = c_loop.close(b, next_c);

    let (mut b, d, d_loop) = b.cell_loop::<u32>();
    let next_d = c
        .steps()
        .merge(&mut b, d.steps().map(|n| n + 1))
        .hold(&mut b, 0);
    let (_b, _d) = d_loop.close(b, next_d);
}
