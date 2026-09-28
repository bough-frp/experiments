//! A countdown with the forward on the defer's output side, the count
//! held, and a second loop totalling the counts. `later` is closed with a
//! defer's output, which depends on nothing in the instant it fires in, so
//! `count` depends on no loop this instant and the total is legal. The
//! forward stays `Instantaneous` in the marker design, so the total is
//! refused: `two-loops-reset`'s coarseness, in the shape a defer loop
//! takes when its forward is the deferred stream.
//@ legal yes
//@ designs baseline marker
#[cfg_attr(design = "marker", path = "api/marker.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let starts = b.input::<u32>();
    let (later, later_loop) = b.stream_loop::<u32>();
    let count = starts.merge(&mut b, later).hold(&mut b, 0);
    let again = count.steps().filter(|n| *n > 1).map(|n| n - 1).defer(&mut b);
    later_loop.close(&mut b, again);

    let (total, total_loop) = b.cell_loop::<u32>();
    let next_total = count
        .steps()
        .snapshot(total, |n, t| t + n)
        .hold(&mut b, 0);
    total_loop.close(&mut b, next_total);
}
