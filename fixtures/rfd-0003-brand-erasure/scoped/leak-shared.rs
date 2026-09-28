//! Wanted to fail: a `map`-like `construct` closure captures an empty shared
//! slot that I/O code fills with an `Anchored` after the build.
#![forbid(unsafe_code)]
#[path = "api.rs"]
mod bough;
use bough::*;
use std::sync::{Arc, Mutex};

fn main() {
    runtime(|mut rt| {
        let slot: Arc<Mutex<Vec<Anchored<'_, Cell<'static, u32>>>>> = Arc::default();
        let in_graph = slot.clone();
        let latest = rt.mutate(move |b| {
            let (n, _) = b.input::<u32>();
            let latest = n.hold(b, 0);
            let (opens, _) = b.input::<u32>();
            let _made = opens.construct(b, move |_, v| v + in_graph.lock().unwrap().len() as u32);
            b.anchor(latest)
        });
        slot.lock().unwrap().push(latest);
    });
}
