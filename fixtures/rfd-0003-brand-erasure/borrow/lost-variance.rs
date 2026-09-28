//! Must fail, recording what the read view gives up: covariance. `&'long A`
//! shortens to `&'short A`; a derived view's fields are projections,
//! `<F as Borrow>::Ref<'a>`, which makes it invariant in `'a`, so a function
//! can't hand back a view at a shorter borrow than it was given.
#![forbid(unsafe_code)]
use bough::*;

#[derive(Rebrand)]
struct Panels<'g> {
    left: Cell<'g, u32>,
    title: String,
}

fn shorten_ref<'short, 'long: 'short, 'g>(p: &'long Panels<'g>) -> &'short Panels<'g> {
    p
}

fn shorten_view<'short, 'long: 'short, 'g>(p: PanelsRef<'long, 'g>) -> PanelsRef<'short, 'g> {
    p
}

fn main() {}
