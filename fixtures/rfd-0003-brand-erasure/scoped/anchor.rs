//! Must build: an anchored input kept across two `mutate`s by I/O code in
//! the runtime's scope, and reopened.
#![forbid(unsafe_code)]
#[path = "api.rs"]
mod bough;
use bough::*;

fn main() {
    runtime(|mut rt| {
        let n_in = rt.mutate(|b| {
            let (_, n_in) = b.input::<u32>();
            b.anchor(n_in)
        });
        rt.mutate(|b| {
            let n_in = b.open(&n_in);
            b.send(n_in, 3);
        });
    });
}
