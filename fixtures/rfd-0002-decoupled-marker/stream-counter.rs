//! A stream loop through a snapshot of a hold of its own forward: the
//! stream-loop spelling of F1.
//@ legal yes
//@ designs baseline marker
#[cfg_attr(design = "marker", path = "api/marker.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<()>();
    let (counts, counts_loop) = b.stream_loop::<u32>();
    let latest = counts.hold(&mut b, 0);
    let next = ticks.snapshot(latest, |_, n| n + 1).filter(|n| *n <= 10);
    counts_loop.close(&mut b, next);
}
