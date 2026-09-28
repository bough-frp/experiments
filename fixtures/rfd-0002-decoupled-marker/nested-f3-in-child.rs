//! Illegal: `nested-loop-in-child` with F3's loop in the body: at the
//! child instant, the counter depends on its own steps.
//@ legal no
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
        let ticks = b.input::<u32>();
        let (c, c_loop) = b.cell_loop::<u32>();
        let bumped = c.steps().map(move |n| n + step);
        let next = ticks.merge(b, bumped).hold(b, 0);
        c_loop.close(b, next);
        next
    });
}
