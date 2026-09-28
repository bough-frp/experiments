//! The navigation loop under rows.
//@ legal yes
//@ designs rows
#[path = "api/rows.rs"]
mod bough;
use bough::*;

pub fn program() {
    let mut b = Build::new();
    let (nav, nav_loop) = b.stream_loop::<u8, L0, Empty>();
    let screen = nav.hold(&mut b, 0);
    let buttons = screen.construct(&mut b, |b, _screen| b.input::<u8>());
    let next = buttons.switch_stream(&mut b);
    nav_loop.close(&mut b, next);
}
