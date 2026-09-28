//! Must build: a `snapshot` of a cell, which the chain traces.
#[path = "api.rs"]
mod bough;
use bough::*;

pub fn program() {
    let mut rt = Runtime::new();
    let _ = rt.mutate(|b| {
        let (n, n_in) = b.input::<u32>();
        let (k, k_in) = b.input::<u32>();
        let latest = k.hold(b, 1);
        let scaled = n.snapshot(latest, |v, k| v * k).hold(b, 0);
        b.anchor((n_in, k_in, scaled))
    });
}
