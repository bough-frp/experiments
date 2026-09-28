//! `nested-loop-reads-parent-forward` in the marker-last design. The
//! closure's build is an epoch past the parent's, so the captured forward
//! is an old one to the watcher's close. Legal.
//@ legal yes
//@ designs baseline marker-last
#[cfg_attr(design = "marker-last", path = "api/marker-last.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline-last.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<()>();
    let spawns = b.input::<()>();
    let (mut b, c, c_loop) = b.cell_loop::<u32>();
    let deferred = spawns.defer(&mut b);
    let spawned = deferred.construct(&mut b, move |b, _| {
        let (mut b, w, w_loop) = b.cell_loop::<u32>();
        let next_w = c.steps().snapshot(w, |n, total| total + n).hold(&mut b, 0);
        let (b, _w) = w_loop.close(b, next_w);
        (b, 1)
    });
    let next_c = ticks
        .map(|_| 1)
        .merge(&mut b, spawned)
        .snapshot(c, |d, n| n + d)
        .hold(&mut b, 0);
    let (_b, _c) = c_loop.close(b, next_c);
}
