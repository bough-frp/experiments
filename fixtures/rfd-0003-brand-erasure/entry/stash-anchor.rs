//! Builds today, and shouldn't: the committed route. The stashed token is
//! anchored in a later `mutate` and opened at its brand: live while the
//! stash's source is rooted, stale after. Under `bough_entry`, `anchor`
//! takes only a value at the `mutate`'s brand.
#![forbid(unsafe_code)]
#[path = "stash.rs"]
mod stash;
use bough::*;

fn main() {
    let mut rt = Runtime::new();
    let (stashed, pair) = stash::stash(&mut rt);
    rt.mutate(move |b| {
        let anchored = b.anchor(stashed);
        println!("anchored, opened:  {:?}", b.sample(b.open(&anchored)));
    });
    drop(pair);
    println!("collected:         {} nodes", rt.collect());
    rt.mutate(move |b| {
        let anchored = b.anchor(stashed);
        println!("after:             {:?}", b.sample(b.open(&anchored)));
    });
}
