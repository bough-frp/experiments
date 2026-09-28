//! Must build for RFD 6: a thread spawned with `std::thread::spawn`, as a
//! tokio task is, holds a `RemoteIo` and an anchored input and sends to it.
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
        let user = thread::spawn(move || remote.send(&joins, 1));
        rt.pump();
        user.join().unwrap();
    });
}
