//! `nested-loop-reads-parent-forward` where the watchers don't feed the
//! parent: the construct is built after the parent closes, so it captures
//! the returned token, and the watcher's loop closes. Legal.
//@ legal yes
//@ designs baseline marker-close
#[cfg_attr(design = "marker-close", path = "api/marker-close.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline-close.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<()>();
    let spawns = b.input::<()>();
    let (c, c_loop) = b.cell_loop::<u32>();
    let next_c = ticks.snapshot(c, |_, n| n + 1).hold(&mut b, 0);
    let c = c_loop.close(&mut b, next_c);
    let _watchers = spawns.defer(&mut b).construct(&mut b, move |b, _| {
        let (w, w_loop) = b.cell_loop::<u32>();
        let next_w = c.steps().snapshot(w, |n, total| total + n).hold(b, 0);
        w_loop.close(b, next_w)
    });
}
