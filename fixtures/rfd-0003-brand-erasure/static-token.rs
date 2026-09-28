//! Must fail: a closure that reaches a token through a thread-local rather
//! than a capture, as in the earlier probe.
#![forbid(unsafe_code)]
#![cfg_attr(bough_auto, feature(auto_traits, negative_impls))]
#[path = "api.rs"]
mod bough;
use bough::*;
use std::cell::Cell as Slot;

thread_local! {
    static DOUBLED: Slot<Option<Cell<'static, u32>>> = const { Slot::new(None) };
}

fn main() {
    let mut rt = Runtime::new();
    let _ = rt.mutate(|b| {
        let (n, n_in) = b.input::<u32>();
        let (pick, pick_in) = b.input::<u32>();
        let latest = n.hold(b, 0);
        DOUBLED.set(Some(latest.map_cell(b, |v| v * 2)));
        let chosen = pick
            .with(latest)
            .map(|(latest, p)| DOUBLED.get().filter(|_| p == 1).unwrap_or(latest))
            .hold(b, latest);
        let shown = chosen.switch_cell(b);
        b.anchor((n_in, pick_in, shown))
    });
}
