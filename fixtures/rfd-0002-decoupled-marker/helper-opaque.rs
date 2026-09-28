//! F1 with its chain in a helper, written as one would without the
//! marker in mind: `impl Source<Event = u32>` in and out. Legal.
//@ legal yes
//@ designs baseline marker
#[cfg_attr(design = "marker", path = "api/marker.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline.rs")]
mod bough;
use bough::*;

fn limited(counts: impl Source<Event = u32>) -> impl Source<Event = u32> {
    counts.filter(|n| *n <= 10)
}

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<()>();
    let (c, c_loop) = b.cell_loop::<u32>();
    let next = limited(ticks.snapshot(c, |_, n| n + 1)).hold(&mut b, 0);
    c_loop.close(&mut b, next);
}
