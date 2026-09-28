//! `defer-forward-feeds-loop` under rows: the total's declaration lists
//! the countdown's slot.
//@ legal yes
//@ designs rows
#[path = "api/rows.rs"]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let starts = b.input::<u32>();
    let (later, later_loop) = b.stream_loop::<u32, L0, Empty>();
    let count = starts.merge(&mut b, later).hold(&mut b, 0);
    let again = count.steps().filter(|n| *n > 1).map(|n| n - 1).defer(&mut b);
    later_loop.close(&mut b, again);

    let (total, total_loop) = b.cell_loop::<u32, L1, Only<L0>>();
    let next_total = count
        .steps()
        .snapshot(total, |n, t| t + n)
        .hold(&mut b, 0);
    total_loop.close(&mut b, next_total);
}
