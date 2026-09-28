//! Illegal: `nested-split-feeds-parent` with the closure's stream merging
//! the split's output with the loop's steps directly, `split-bypass`
//! built at a child instant. Once the switch selects that stream, the
//! loop depends on its own steps in the same instant.
//@ legal no
//@ designs baseline marker marker-close
#[cfg_attr(design = "marker", path = "api/marker.rs")]
#[cfg_attr(design = "marker-close", path = "api/marker-close.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<u32>();
    let spawns = b.input::<()>();
    let (c, c_loop) = b.cell_loop::<u32>();
    let spawned = spawns.defer(&mut b).hold(&mut b, ());
    let screen = spawned.construct(&mut b, move |b, _| {
        let later = c.steps().filter(|n| *n > 0).map(|n| vec![n - 1]).split(b);
        later.merge(b, c.steps().map(|n| n + 1))
    });
    let rounds = screen.switch_stream(&mut b);
    let next = ticks.merge(&mut b, rounds).hold(&mut b, 0);
    c_loop.close(&mut b, next);
}
