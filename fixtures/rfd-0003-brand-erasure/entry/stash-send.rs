//! Builds today, and shouldn't: the stashed token sent as an event, through
//! `Build::send`, to an input whose event type holds it at `'static`. The
//! held cell anchors as `Cell<'static, Option<Cell<'static, u32>>>`, so a
//! later `open` renames both brands to that `mutate`'s, and the event reads
//! live. Under `bough_entry`, `send` and `anchor` take only a value at the
//! `mutate`'s brand, and this stream has two.
#![forbid(unsafe_code)]
#[path = "stash.rs"]
mod stash;
use bough::*;

fn main() {
    let mut rt = Runtime::new();
    let (stashed, _pair) = stash::stash(&mut rt);
    let held = rt.mutate(move |b| {
        let (sent, sent_in) = b.input::<Option<Cell<'static, u32>>>();
        let held = sent.hold(b, None);
        b.send(sent_in, Some(stashed));
        b.anchor(held)
    });
    rt.mutate(|b| {
        let got = b.sample(b.open(&held)).expect("live").expect("sent");
        println!("sent, held, opened: {:?}", b.sample(got));
    });
}
