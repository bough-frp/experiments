//! Illegal: F3 inside a `construct` closure in the marker-last design.
//! The child's own forward is of the child's epoch.
//@ legal no
//@ designs baseline marker-last
#[cfg_attr(design = "marker-last", path = "api/marker-last.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline-last.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let jobs = b.input::<Vec<u32>>();
    let _counters = jobs.split(&mut b).construct(&mut b, |mut b, step: u32| {
        let ticks = b.input::<u32>();
        let (mut b, c, c_loop) = b.cell_loop::<u32>();
        let bumped = c.steps().map(move |n| n + step);
        let next = ticks.merge(&mut b, bumped).hold(&mut b, 0);
        c_loop.close(b, next)
    });
}
