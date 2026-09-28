//! Must fail, in every design: a stashed token used as it is, in a later
//! `mutate`. It is `Cell<'static, u32>`, and the `mutate` reads at its own
//! brand, which `'static` isn't.
#![forbid(unsafe_code)]
#[path = "stash.rs"]
mod stash;
use bough::*;

fn main() {
    let mut rt = Runtime::new();
    let (stashed, _pair) = stash::stash(&mut rt);
    rt.mutate(move |b| {
        println!("{:?}", b.sample(stashed));
    });
}
