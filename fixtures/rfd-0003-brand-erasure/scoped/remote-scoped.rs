//! Must build: the same send from a scoped thread, which may borrow from the
//! runtime's scope.
#![forbid(unsafe_code)]
#[path = "api.rs"]
mod bough;
use bough::*;
use std::thread;

fn main() {
    runtime(|mut rt| {
        let joins = rt.mutate(|b| {
            let (_, joins_in) = b.input::<u32>();
            b.anchor(joins_in)
        });
        let remote = rt.remote_io();
        thread::scope(|s| {
            s.spawn(|| remote.send(&joins, 1));
            rt.pump();
        });
    });
}
