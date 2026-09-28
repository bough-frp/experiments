//! The navigation loop in the marker-last design: the `construct`
//! closure takes the child build and hands it back.
//@ legal yes
//@ designs baseline marker-last
#[cfg_attr(design = "marker-last", path = "api/marker-last.rs")]
#[cfg_attr(design = "baseline", path = "api/baseline-last.rs")]
mod bough;
use bough::*;

pub fn program() {
    let b = Build::new();
    let (mut b, nav, nav_loop) = b.stream_loop::<u8>();
    let screen = nav.hold(&mut b, 0);
    let buttons = screen.construct(&mut b, |mut b, _screen| {
        let buttons = b.input::<u8>();
        (b, buttons)
    });
    let next = buttons.switch_stream(&mut b);
    let (_b, _nav) = nav_loop.close(b, next);
}
