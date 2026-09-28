//! The navigation loop with tabs: each screen's buttons are a switch over
//! its tabs' buttons, so the outer switch's inner is another switch's
//! output. Nothing reaches back: legal. A design that keeps a switch's
//! output from being an inner, so that no inner can be its own switch's
//! consumer, refuses it.
//@ legal yes
//@ designs baseline marker
#[cfg_attr(design = "marker", path = "api/marker.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let (nav, nav_loop) = b.stream_loop::<u8>();
    let screen = nav.hold(&mut b, 0);
    let buttons = screen.construct(&mut b, |b, _screen| {
        let tab = b.input::<u8>().hold(b, 0);
        let tab_buttons = tab.construct(b, |b, _tab| b.input::<u8>());
        tab_buttons.switch_stream(b)
    });
    let next = buttons.switch_stream(&mut b);
    nav_loop.close(&mut b, next);
}
