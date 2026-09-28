//! F1 in the marker-last design: the loop takes and returns the build.
//@ legal yes
//@ designs baseline marker-last
#[cfg_attr(design = "marker-last", path = "api/marker-last.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline-last.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<()>();
    let (mut b, c, c_loop) = b.cell_loop::<u32>();
    let next = ticks
        .snapshot(c, |_, n| n + 1)
        .filter(|n| *n <= 10)
        .hold(&mut b, 0);
    let (_b, _c) = c_loop.close(b, next);
}
