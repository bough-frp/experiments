//! A loop inside a `construct` body reading the parent loop's steps. The
//! parent counts ticks and spawns; each spawn, at `t ++ [0]`, builds a
//! watcher that totals the parent's steps and adds one to the count. The
//! construct feeds the parent's definition, so it is built before the
//! parent closes and captures the forward; by the time the closure runs
//! the parent has long closed. The watcher depends on the parent in the
//! same instant, the parent on the watcher not at all: acyclic, legal.
//@ legal yes
//@ designs baseline marker marker-close
#[cfg_attr(design = "marker", path = "api/marker.rs")]
#[cfg_attr(design = "marker-close", path = "api/marker-close.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<()>();
    let spawns = b.input::<()>();
    let (c, c_loop) = b.cell_loop::<u32>();
    let spawned = spawns.defer(&mut b).construct(&mut b, move |b, _| {
        let (w, w_loop) = b.cell_loop::<u32>();
        let next_w = c.steps().snapshot(w, |n, total| total + n).hold(b, 0);
        w_loop.close(b, next_w);
        1
    });
    let next_c = ticks
        .map(|_| 1)
        .merge(&mut b, spawned)
        .snapshot(c, |d, n| n + d)
        .hold(&mut b, 0);
    c_loop.close(&mut b, next_c);
}
