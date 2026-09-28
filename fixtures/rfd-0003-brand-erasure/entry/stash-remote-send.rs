//! Builds today, and shouldn't: `stash-runtime-send.rs` through a
//! `RemoteIo` on another thread, which queues the send for the next `pump`.
//! Under `bough_entry`, `RemoteIo::send` takes only a value with no token in
//! it.
#![forbid(unsafe_code)]
#[path = "stash.rs"]
mod stash;
use bough::*;

fn main() {
    let mut rt = Runtime::new();
    let (stashed, _pair) = stash::stash(&mut rt);
    let (input, held) = rt.mutate(|b| {
        let (sent, sent_in) = b.input::<Option<Cell<'_, u32>>>();
        let held = sent.hold(b, None);
        (b.anchor(sent_in), b.anchor(held))
    });
    let remote = rt.remote_io();
    std::thread::spawn(move || remote.send(&input, Some(stashed)))
        .join()
        .expect("the sender");
    rt.pump();
    rt.mutate(|b| {
        let got = b.sample(b.open(&held)).expect("live").expect("sent");
        println!("sent from a thread, read live: {:?}", b.sample(got));
    });
}
