//! F1 with the counter in a helper that takes the cell, written as one
//! would without the marker in mind: `Cell<u32>`. Legal.
//@ legal yes
//@ designs baseline marker
#[cfg_attr(design = "marker", path = "api/marker.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline.rs")]
mod bough;
use bough::*;

fn counter(b: &mut Build, ticks: Stream<()>, c: Cell<u32>) -> Cell<u32> {
    ticks
        .snapshot(c, |_, n| n + 1)
        .filter(|n| *n <= 10)
        .hold(b, 0)
}

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<()>();
    let (c, c_loop) = b.cell_loop::<u32>();
    let next = counter(&mut b, ticks, c);
    c_loop.close(&mut b, next);
}
