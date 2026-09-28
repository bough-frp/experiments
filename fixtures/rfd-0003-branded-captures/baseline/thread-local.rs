//! Must fail: a closure that reaches a token through a thread-local rather
//! than a capture, so neither `depends` nor a capture rule sees it.
#[path = "api.rs"]
mod bough;
use bough::*;
use std::cell::Cell as Slot;

thread_local! {
    static DOUBLED: Slot<Option<Cell<u32>>> = const { Slot::new(None) };
}

pub fn program() {
    let mut rt = Runtime::new();
    let _ = rt.build(|b| {
        let (n, n_in) = b.input::<u32>();
        let (pick, pick_in) = b.input::<u32>();
        let latest = n.hold(b, 0);
        DOUBLED.set(Some(latest.map_cell(b, |v| v * 2)));
        let chosen = pick
            .map(move |p| DOUBLED.get().filter(|_| p == 1).unwrap_or(latest))
            .hold(b, latest);
        b.depends(&chosen, &[&latest]);
        (n_in, pick_in, chosen.switch_cell(b))
    });
}
