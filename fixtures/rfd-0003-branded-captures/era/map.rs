//! Must build: a `map` over an input, held.
#[path = "api.rs"]
mod bough;
use bough::*;

pub fn program() {
    let mut rt = Runtime::new();
    let _ = rt.build(|b| {
        let (n, n_in) = b.input::<u32>();
        let doubled = n.map(|v| v * 2).hold(b, 0);
        (n_in, doubled)
    });
}
