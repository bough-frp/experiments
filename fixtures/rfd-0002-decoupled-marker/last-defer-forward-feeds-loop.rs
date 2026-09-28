//! `defer-forward-feeds-loop` in the marker-last design, as written:
//! the total reads `count`, a hold built from the countdown's forward
//! before its close. That close leaves no loop open, so the total's loop
//! opens in the next epoch, where `count`'s mark is an old one. Legal.
//@ legal yes
//@ designs baseline marker-last
#[cfg_attr(design = "marker-last", path = "api/marker-last.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline-last.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let starts = b.input::<u32>();
    let (mut b, later, later_loop) = b.stream_loop::<u32>();
    let count = starts.merge(&mut b, later).hold(&mut b, 0);
    let again = count
        .steps()
        .filter(|n| *n > 1)
        .map(|n| n - 1)
        .defer(&mut b);
    let (b, _later) = later_loop.close(b, again);

    let (mut b, total, total_loop) = b.cell_loop::<u32>();
    let next_total = count.steps().snapshot(total, |n, t| t + n).hold(&mut b, 0);
    let (_b, _total) = total_loop.close(b, next_total);
}
