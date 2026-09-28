//! Must fail: a hand-written `Rebrand` whose `Of` keeps the old brand, so
//! `open` would hand back the token of the `mutate` that anchored it.
#![forbid(unsafe_code)]
#![cfg_attr(bough_auto, feature(auto_traits, negative_impls))]
#[path = "api.rs"]
mod bough;
use bough::*;

#[derive(Clone, Copy)]
struct Clicks<'g> {
    clicks: Shared<'g, u32>,
}

impl Trace for Clicks<'_> {
    fn trace(&self, tracer: &mut Tracer) {
        self.clicks.trace(tracer);
    }
}

impl<'g> Rebrand for Clicks<'g> {
    type Of<'x> = Clicks<'g>;
    fn rebrand<'x>(&self, _: Witness<'x>) -> Clicks<'g> {
        *self
    }
    fn restore<'x>(from: &Clicks<'g>, _: Witness<'x>) -> Self {
        *from
    }
}

fn main() {
    let mut rt = Runtime::new();
    let _ = rt.mutate(|b| {
        let (clicks, clicks_in) = b.input::<u32>();
        let clicks = clicks.share(b);
        b.anchor((clicks_in, Clicks { clicks }))
    });
}
