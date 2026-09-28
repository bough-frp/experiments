//! `defer-forward-feeds-loop` with the forward on the defer's input side,
//! as `defer-countdown` has it: the count is a hold of the merge with the
//! deferred stream, and the loop closes with its steps. Same graph; the
//! count is `Decoupled`, so the total closes in the marker design too.
//@ legal yes
//@ designs baseline marker
#[cfg_attr(design = "marker", path = "api/marker.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let starts = b.input::<u32>();
    let (counts, counts_loop) = b.stream_loop::<u32>();
    let again = counts.filter(|n| *n > 1).map(|n| n - 1).defer(&mut b);
    let count = starts.merge(&mut b, again).hold(&mut b, 0);
    counts_loop.close(&mut b, count.steps());

    let (total, total_loop) = b.cell_loop::<u32>();
    let next_total = count
        .steps()
        .snapshot(total, |n, t| t + n)
        .hold(&mut b, 0);
    total_loop.close(&mut b, next_total);
}
