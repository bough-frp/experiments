//! Wanted to fail: a `construct` closure captures an `Anchored` and
//! reopens it (question 3).
#![forbid(unsafe_code)]
#[path = "api.rs"]
mod bough;
use bough::*;

fn main() {
    runtime(|mut rt| {
        rt.mutate(|b| {
            let (shown, _) = b.input::<u32>();
            let shown = shown.hold(b, 0);
            let shown = b.anchor(shown);
            let (opens, _) = b.input::<u32>();
            let _made = opens.construct(b, move |b, _| b.open(&shown));
        });
    });
}
