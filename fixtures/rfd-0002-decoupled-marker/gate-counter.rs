//! A counter that stops at ten through `gate`:
//! `c = hold 0 (snapshot (\_ n -> n + 1) (gate ticks (map (< 10) c)) c)`.
//! The gate reads a read-through cell of the forward, from before the
//! instant, so only the snapshot and the gate touch the loop.
//@ legal yes
//@ designs baseline marker
#[cfg_attr(design = "marker", path = "api/marker.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<()>();
    let (c, c_loop) = b.cell_loop::<u32>();
    let below_ten = c.map_cell(&mut b, |n| *n < 10);
    let next = ticks
        .gate(below_ten)
        .snapshot(c, |_, n| n + 1)
        .hold(&mut b, 0);
    c_loop.close(&mut b, next);
}
