//! `nested-loop-in-child` in the marker-last design: a counter per item,
//! built in the child build the closure gets and hands back. Legal.
//@ legal yes
//@ designs baseline marker-last
#[cfg_attr(design = "marker-last", path = "api/marker-last.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline-last.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let jobs = b.input::<Vec<u32>>();
    let _counters = jobs.split(&mut b).construct(&mut b, |mut b, step: u32| {
        let ticks = b.input::<()>();
        let (mut b, c, c_loop) = b.cell_loop::<u32>();
        let next = ticks.snapshot(c, move |_, n| n + step).hold(&mut b, 0);
        c_loop.close(b, next)
    });
}
