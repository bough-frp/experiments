//! Wanted to fail: question 3. A `map` closure captures an empty shared
//! slot that I/O code fills with an `Anchored` after the build: the guard
//! is hidden in graph state through interior mutability, where no bound on
//! `Anchored` itself can see it at the capture.
#![forbid(unsafe_code)]
#![cfg_attr(bough_auto, feature(auto_traits, negative_impls))]
#[path = "api.rs"]
mod bough;
use bough::*;
use std::sync::{Arc, Mutex};

fn main() {
    let mut rt = Runtime::new();
    let slot: Arc<Mutex<Vec<Anchored<Cell<'static, u32>>>>> = Arc::default();
    let in_graph = slot.clone();
    let latest = rt.mutate(move |b| {
        let (n, n_in) = b.input::<u32>();
        let n = n.share(b);
        let latest = n.hold(b, 0);
        let kept = n
            .map(move |v| v + in_graph.lock().unwrap().len() as u32)
            .hold(b, 0);
        (b.anchor((n_in, kept)), b.anchor(latest))
    });
    slot.lock().unwrap().push(latest.1);
}
