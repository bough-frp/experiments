//! `defer-forward-feeds-loop` in the marker-close design, threading what
//! `close` returns. The loop's forward is the deferred stream, and what
//! the total reads is `count`, a hold built from the forward before the
//! close, because the close's definition is built from `count`. The close
//! re-marks the loop's stream, not `count`, and nothing built after the
//! close can stand in for `count`: `starts` is spent. Legal, as before.
//@ legal yes
//@ designs baseline marker-close
#[cfg_attr(design = "marker-close", path = "api/marker-close.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline-close.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let starts = b.input::<u32>();
    let (later, later_loop) = b.stream_loop::<u32>();
    let count = starts.merge(&mut b, later).hold(&mut b, 0);
    let again = count
        .steps()
        .filter(|n| *n > 1)
        .map(|n| n - 1)
        .defer(&mut b);
    let _later = later_loop.close(&mut b, again);

    let (total, total_loop) = b.cell_loop::<u32>();
    let next_total = count.steps().snapshot(total, |n, t| t + n).hold(&mut b, 0);
    let _total = total_loop.close(&mut b, next_total);
}
