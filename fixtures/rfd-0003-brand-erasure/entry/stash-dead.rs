//! Must build, in every design: the stash itself. A hand-written `view`
//! copies a `'static` token out; with every entry bounded, this is all it
//! can do with it: read the arena slot it names, as `index` prints it.
#![forbid(unsafe_code)]
#[path = "stash.rs"]
mod stash;
use bough::*;

fn main() {
    let mut rt = Runtime::new();
    let (stashed, pair) = stash::stash(&mut rt);
    println!("stashed:   a token naming slot {}", stashed.index());
    drop(pair);
    println!("collected: {} nodes", rt.collect());
    let reused = rt.mutate(|b| {
        let (n, _) = b.input::<u32>();
        n.hold(b, 9).index()
    });
    println!(
        "a new hold reuses slot {reused}; the stash still names slot {}",
        stashed.index()
    );
}
