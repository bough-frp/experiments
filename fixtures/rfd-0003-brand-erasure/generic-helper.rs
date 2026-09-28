//! Must build: generic helpers. One captures a value of a type parameter,
//! written with the bounds today's closures need; one is generic over the
//! event, which now needs `Rebrand` where it needed nothing.
#![forbid(unsafe_code)]
#![cfg_attr(bough_auto, feature(auto_traits, negative_impls))]
#[path = "api.rs"]
mod bough;
use bough::*;

fn apply<'g, S: Source<'g, Event = u32>, T: Fn(u32) -> u32 + 'static>(
    s: S,
    t: T,
) -> Stream<'g, u32> {
    s.map(move |v| t(v))
}

fn latest<'g, A: Rebrand + Trace, S: Source<'g, Event = A>>(
    b: &mut Build<'g>,
    s: S,
    init: A,
) -> Cell<'g, A> {
    s.hold(b, init)
}

fn main() {
    let mut rt = Runtime::new();
    let _ = rt.mutate(|b| {
        let (n, n_in) = b.input::<u32>();
        let plus = apply(n, |v| v + 1);
        let plus = latest(b, plus, 0);
        b.anchor((n_in, plus))
    });
}
