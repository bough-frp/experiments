//! F1, the counter that stops at ten:
//! `c = hold 0 (filter (<= 10) (snapshot (\_ n -> n + 1) ticks c))`.
//! The loop reads `c` only from before the instant.
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
    let next = ticks
        .snapshot(c, |_, n| n + 1)
        .filter(|n| *n <= 10)
        .hold(&mut b, 0);
    c_loop.close(&mut b, next);
}
