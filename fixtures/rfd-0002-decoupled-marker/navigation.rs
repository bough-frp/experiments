//! The navigation loop: the screen is a hold of the navigation events,
//! each screen is built with its own button stream, and the navigation
//! events are a `switch_stream` over the screen's buttons. The loop crosses
//! the switch's selection, which is read from before the instant.
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
    let buttons = screen.construct(&mut b, |b, _screen| b.input::<u8>());
    let next = buttons.switch_stream(&mut b);
    nav_loop.close(&mut b, next);
}
