//! Must build: a `construct` that uses three tokens, declared by passing
//! them through `with` rather than capturing them.
#![feature(auto_traits, negative_impls)]
#[path = "api.rs"]
mod bough;
use bough::*;

pub fn program() {
    let mut rt = Runtime::new();
    let _ = rt.build(|b| {
        let (clicks, clicks_in) = b.input::<u32>();
        let clicks = clicks.share(b);
        let scale = clicks.hold(b, 1);
        let offset = clicks.map(|c| c + 10).hold(b, 10);
        let (opens, opens_in) = b.input::<u32>();
        let made =
            opens
                .with((clicks, scale, offset))
                .construct(b, |b, ((clicks, scale, offset), id)| {
                    clicks
                        .snapshot(scale, move |c, s| id + c * s)
                        .snapshot(offset, |v, o| v + o)
                        .hold(b, 0)
                });
        let made = made.hold(b, scale);
        (clicks_in, opens_in, made)
    });
}
