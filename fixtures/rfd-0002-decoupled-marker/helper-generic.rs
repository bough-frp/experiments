//! `helper-opaque` and `helper-token` with the signatures the marker
//! needs: the chain helper names its mark, the token helper is generic
//! over it. Legal. Marker only, since the baseline has no `Mark`.
//@ legal yes
//@ designs marker
#[path = "api/marker.rs"]
mod bough;
use bough::*;

fn limited<S: Source<Event = u32>>(counts: S) -> impl Source<Event = u32, Mark = S::Mark> {
    counts.filter(|n| *n <= 10)
}

fn counter<M: Mark>(b: &mut Build, ticks: Stream<()>, c: Cell<u32, M>) -> Cell<u32> {
    ticks
        .snapshot(c, |_, n| n + 1)
        .filter(|n| *n <= 10)
        .hold(b, 0)
}

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<()>();
    let (c, c_loop) = b.cell_loop::<u32>();
    let next = limited(ticks.snapshot(c, |_, n| n + 1)).hold(&mut b, 0);
    c_loop.close(&mut b, next);

    let ticks = b.input::<()>();
    let (d, d_loop) = b.cell_loop::<u32>();
    let next = counter(&mut b, ticks, d);
    d_loop.close(&mut b, next);
}
