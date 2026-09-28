//! A counter through `sample`: each tick runs a `construct` closure that
//! samples the loop's cell, from before the instant, and emits one more.
//! The closure captures the forward, but what it returns is a value.
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
        .construct(&mut b, move |b, _| *c.sample(b) + 1)
        .hold(&mut b, 0);
    c_loop.close(&mut b, next);
}
