//! F1's counter in a helper that opens and closes its own loop, called
//! once with no loop open and once with one open. The helper threads the
//! build and names the epoch its close leaves: the next one if it closed
//! the last open loop, the same one otherwise. Legal. Marker-last only,
//! since the signature names its type-state.
//@ legal yes
//@ designs marker-last
#[path = "api/marker-last.rs"]
mod bough;
use bough::*;

fn counter<E: 'static, N: Open>(
    b: Build<E, N>,
    ticks: Stream<()>,
) -> (Build<N::After<E>, N>, Cell<u32>) {
    let (mut b, c, c_loop) = b.cell_loop::<u32>();
    let next = ticks
        .snapshot(c, |_, n| n + 1)
        .filter(|n| *n <= 10)
        .hold(&mut b, 0);
    c_loop.close(b, next)
}

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<()>();
    let (b, first) = counter(b, ticks);

    let (mut b, total, total_loop) = b.cell_loop::<u32>();
    let ticks = b.input::<()>();
    let (mut b, second) = counter(b, ticks);
    let next = first
        .steps()
        .merge(&mut b, second.steps())
        .snapshot(total, |n, t| t + n)
        .hold(&mut b, 0);
    let (_b, _total) = total_loop.close(b, next);
}
