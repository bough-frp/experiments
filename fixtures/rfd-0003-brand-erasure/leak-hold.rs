//! Wanted to fail: question 3. A `map` closure captures an `Anchored` and
//! never opens it: still a guard hidden in graph state.
#![forbid(unsafe_code)]
#![cfg_attr(bough_auto, feature(auto_traits, negative_impls))]
#[path = "api.rs"]
mod bough;
use bough::*;

fn main() {
    let mut rt = Runtime::new();
    let _ = rt.mutate(|b| {
        let (n, n_in) = b.input::<u32>();
        let n = n.share(b);
        let latest = n.hold(b, 0);
        let latest = b.anchor(latest);
        let kept = n
            .map(move |v| {
                let _root = &latest;
                v + 1
            })
            .hold(b, 0);
        b.anchor((n_in, kept))
    });
}
