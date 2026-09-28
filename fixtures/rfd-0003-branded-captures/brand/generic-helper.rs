//! Must build: a generic helper that captures a value of a type parameter,
//! written with the bounds today's closures need, plus the brand.
#[path = "api.rs"]
mod bough;
use bough::*;

fn apply<'g, S: Source<'g, Event = u32>, T: Fn(u32) -> u32 + 'static>(
    s: S,
    t: T,
) -> Stream<'g, u32> {
    s.map(move |v| t(v))
}

pub fn program() {
    let mut rt = Runtime::new();
    let _ = rt.mutate(|b| {
        let (n, n_in) = b.input::<u32>();
        let plus = apply(n, |v| v + 1).hold(b, 0);
        b.anchor((n_in, plus))
    });
}
