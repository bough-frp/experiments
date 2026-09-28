//! Builds today, and shouldn't: the stashed token sent from outside any
//! `mutate`, with `Runtime::send`, to a legal input of tokens. A stored
//! value's tokens are at `'static`, so the stashed one is already in the
//! stored form, and a later `sample` reads it live. Under `bough_entry`,
//! `Runtime::send` takes only a value with no token in it.
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
    rt.send(&input, Some(stashed));
    rt.mutate(|b| {
        let got = b.sample(b.open(&held)).expect("live").expect("sent");
        println!("sent from outside, read live: {:?}", b.sample(got));
    });
}
