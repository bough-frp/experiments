//! A loop entirely inside a `construct` body that runs at a child
//! instant: each job splits into items, each item at `t ++ [n]`, and the
//! closure builds a counter for it there, F1's shape. Legal.
//@ legal yes
//@ designs baseline marker marker-close
#[cfg_attr(design = "marker", path = "api/marker.rs")]
#[cfg_attr(design = "marker-close", path = "api/marker-close.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let jobs = b.input::<Vec<u32>>();
    let _counters = jobs.split(&mut b).construct(&mut b, |b, step: u32| {
        let ticks = b.input::<()>();
        let (c, c_loop) = b.cell_loop::<u32>();
        let next = ticks.snapshot(c, move |_, n| n + step).hold(b, 0);
        c_loop.close(b, next);
        next
    });
}
