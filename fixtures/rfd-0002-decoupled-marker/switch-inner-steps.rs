//! Illegal: the navigation loop with a screen whose inner stream is the
//! screen's own steps. The switch's output depends on the inner it selects
//! this instant, and that inner depends on the loop.
//@ legal no
//@ designs baseline marker
#[cfg_attr(design = "marker", path = "api/marker.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline.rs")]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let (nav, nav_loop) = b.stream_loop::<u8>();
    let screen = nav.hold(&mut b, 0);
    let buttons = screen.construct(&mut b, move |_, _| screen.steps());
    let next = buttons.switch_stream(&mut b);
    nav_loop.close(&mut b, next);
}
