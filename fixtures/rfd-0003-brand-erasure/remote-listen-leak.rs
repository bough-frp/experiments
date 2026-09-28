//! Must fail: a `RemoteIo` listener keeps the token its row holds, in a
//! channel of `'static` tokens, rather than anchoring it.
#![forbid(unsafe_code)]
#![cfg_attr(bough_auto, feature(auto_traits, negative_impls))]
#[path = "api.rs"]
mod bough;
use bough::*;
use std::sync::mpsc;

fn main() {
    let mut rt = Runtime::new();
    let rows = rt.mutate(|b| {
        let (votes, votes_in) = b.input::<u32>();
        let _total = votes.accumulate(b, 0u32, |v, t| t + v);
        let (asks, _) = b.input::<String>();
        let rows = asks
            .with(votes_in)
            .map(|(input, who)| (who, input))
            .share(b);
        b.anchor(rows)
    });
    let (to_worker, _from_listener) = mpsc::channel::<Input<'static, u32>>();
    rt.remote_io()
        .listen(&rows, move |(_, input)| {
            let _ = to_worker.send(input);
        })
        .keep();
}
