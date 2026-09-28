//! Must fail: a hand-written `Rebrand` impl, which does receive a witness,
//! stashes the token it rebrands in a thread-local, to be picked up by a
//! graph closure in a later `mutate`.
#![forbid(unsafe_code)]
#![cfg_attr(bough_auto, feature(auto_traits, negative_impls))]
#[path = "api.rs"]
mod bough;
use bough::*;
use std::cell::Cell as Slot;

thread_local! {
    static STASH: Slot<Option<Shared<'static, u32>>> = const { Slot::new(None) };
}

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
    type Of<'x> = Clicks<'x>;
    fn rebrand<'x>(&self, to: Witness<'x>) -> Clicks<'x> {
        let out = Clicks {
            clicks: self.clicks.rebrand(to),
        };
        STASH.set(Some(out.clicks));
        out
    }
    fn restore<'x>(from: &Clicks<'x>, at: Witness<'x>) -> Self {
        Clicks {
            clicks: Rebrand::restore(&from.clicks, at),
        }
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
