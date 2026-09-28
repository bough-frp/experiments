//! Illegal: a watcher built in a `construct` closure reads the parent
//! loop's steps, closes, and its returned token's steps go back to the
//! parent as a switch's inner. Once the switch selects it, the parent
//! depends on the watcher and the watcher on the parent, in one instant.
//! The child's epoch lets the watcher close, and its returned token is
//! `Decoupled`: the switch hole, reached through a child build.
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
    let screen = spawned.construct(&mut b, move |b, _| {
        let (mut b, w, w_loop) = b.cell_loop::<u32>();
        let next_w = c.steps().snapshot(w, |n, total| total + n).hold(&mut b, 0);
        let (b, w) = w_loop.close(b, next_w);
        (b, w.steps())
    });
    let from_child = screen.switch_stream(&mut b);
    let next = ticks.merge(&mut b, from_child).hold(&mut b, 0);
    let (_b, _c) = c_loop.close(b, next);
}
