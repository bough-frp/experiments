//! A split built inside a `construct` whose closure runs at a child
//! instant, feeding a loop at the parent. `spawned` steps in `t ++ [0]`,
//! a defer's child, and its `construct` builds a split of the loop's
//! steps there; a `switch_stream` brings the split's output back into the
//! loop. Every path around the loop crosses the split, whose output
//! depends on nothing in the instant it fires in, so the graph stays
//! acyclic. Legal; each round counts down, so it ends.
//@ legal yes
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
        c.steps().filter(|n| *n > 0).map(|n| vec![n - 1]).split(b)
    });
    let rounds = screen.switch_stream(&mut b);
    let next = ticks.merge(&mut b, rounds).hold(&mut b, 0);
    c_loop.close(&mut b, next);
}
