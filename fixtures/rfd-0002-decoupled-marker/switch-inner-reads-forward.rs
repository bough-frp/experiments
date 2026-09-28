//! A screen that shows a counter's steps, built while the counter's loop
//! is still open, so the inner is the forward's steps. The switch's
//! output feeds nothing the counter reads: legal. A design that requires
//! `Decoupled` inners refuses it.
//@ legal yes
//@ designs baseline marker
#[cfg_attr(design = "marker", path = "api/marker.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let ticks = b.input::<()>();
    let pages = b.input::<u8>();
    let (c, c_loop) = b.cell_loop::<u32>();
    let page = pages.hold(&mut b, 0);
    let shown = page.construct(&mut b, move |b, _page| {
        c.steps().map(|n| n * 2).hold(b, 0).steps()
    });
    let _display = shown.switch_stream(&mut b);
    let next = ticks.snapshot(c, |_, n| n + 1).hold(&mut b, 0);
    c_loop.close(&mut b, next);
}
