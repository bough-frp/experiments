//! Must fail: I/O code hands a branded input to another thread directly,
//! rather than anchoring it.
#![forbid(unsafe_code)]
#![cfg_attr(bough_auto, feature(auto_traits, negative_impls))]
#[path = "api.rs"]
mod bough;
use bough::*;
use std::thread;

fn main() {
    let mut rt = Runtime::new();
    let remote = rt.remote_io();
    rt.mutate(move |b| {
        let (_, votes_in) = b.input::<u32>();
        let _ = remote;
        thread::spawn(move || {
            drop(votes_in);
        });
    });
}
