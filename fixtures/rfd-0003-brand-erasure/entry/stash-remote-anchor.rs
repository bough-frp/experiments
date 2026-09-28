//! Builds today, and shouldn't: the stashed token anchored through a
//! `RemoteIo`, the way a listener anchors a row, and opened live after the
//! `pump`; the handle's root keeps it live after the stash's source is
//! unrooted. Under `bough_entry` the handle's bound names a brand of the
//! caller's choosing, and `'static` is one. Under `bough_entry_at` the
//! brand must be proved by an `At`, which only a listener is handed, at its
//! event's fresh brand: this tries the stash at a listener's `At`.
#![forbid(unsafe_code)]
#[path = "stash.rs"]
mod stash;
use bough::*;

#[cfg(not(bough_entry_at))]
fn main() {
    let mut rt = Runtime::new();
    let (stashed, pair) = stash::stash(&mut rt);
    let remote = rt.remote_io();
    let anchored = std::thread::spawn(move || remote.anchor(stashed))
        .join()
        .expect("the anchoring thread");
    rt.pump();
    rt.mutate(|b| println!("anchored from a thread: {:?}", b.sample(b.open(&anchored))));
    drop(pair);
    println!("collected:              {} nodes", rt.collect());
    rt.mutate(|b| println!("after:                  {:?}", b.sample(b.open(&anchored))));
}

#[cfg(bough_entry_at)]
fn main() {
    use std::sync::mpsc;
    let mut rt = Runtime::new();
    let (stashed, _pair) = stash::stash(&mut rt);
    let (pings_in, pings) = rt.mutate(|b| {
        let (pings, pings_in) = b.input::<u32>();
        let pings = pings.share(b);
        (b.anchor(pings_in), b.anchor(pings))
    });
    let remote = rt.remote_io();
    let handle = remote.clone();
    let (tx, rx) = mpsc::channel();
    remote
        .listen(&pings, move |_, at| {
            let _ = tx.send(handle.anchor(at, stashed));
        })
        .keep();
    rt.pump();
    rt.send(&pings_in, 1);
    let anchored = rx.recv().expect("anchored");
    rt.pump();
    rt.mutate(|b| {
        println!(
            "anchored at a listener's At: {:?}",
            b.sample(b.open(&anchored))
        )
    });
}
