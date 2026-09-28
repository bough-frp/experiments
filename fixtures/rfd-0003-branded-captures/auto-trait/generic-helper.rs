//! Must build: a generic helper that captures a value of a type parameter,
//! written with the bounds today's closures need.
#![feature(auto_traits, negative_impls)]
#[path = "api.rs"]
mod bough;
use bough::*;

fn apply<S: Source<Event = u32>, T: Fn(u32) -> u32 + 'static>(s: S, t: T) -> Stream<u32> {
    s.map(move |v| t(v))
}

pub fn program() {
    let mut rt = Runtime::new();
    let _ = rt.build(|b| {
        let (n, n_in) = b.input::<u32>();
        let plus = apply(n, |v| v + 1).hold(b, 0);
        (n_in, plus)
    });
}
