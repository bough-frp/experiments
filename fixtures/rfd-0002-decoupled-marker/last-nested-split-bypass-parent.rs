//! Illegal: `nested-split-bypass-parent` in the marker-last design. The
//! closure returns a stream built from the parent's forward; its type keeps
//! the parent's epoch, so the parent's close still refuses it.
//@ legal no
//@ designs baseline marker-last
#[cfg_attr(design = "marker-last", path = "api/marker-last.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline-last.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<u32>();
    let spawns = b.input::<()>();
    let (mut b, c, c_loop) = b.cell_loop::<u32>();
    let spawned = spawns.defer(&mut b).hold(&mut b, ());
    let screen = spawned.construct(&mut b, move |mut b, _| {
        let later = c
            .steps()
            .filter(|n| *n > 0)
            .map(|n| vec![n - 1])
            .split(&mut b);
        let rounds = later.merge(&mut b, c.steps().map(|n| n + 1));
        (b, rounds)
    });
    let rounds = screen.switch_stream(&mut b);
    let next = ticks.merge(&mut b, rounds).hold(&mut b, 0);
    let (_b, _c) = c_loop.close(b, next);
}
