//! Must build and run, under `bough_entry_at`: `../remote-listen.rs`, RFD
//! 6's listener that anchors a row through a handle, with the `At` its
//! event comes with passed to `anchor`.
#![forbid(unsafe_code)]
use bough::*;
use std::sync::mpsc;
use std::thread;

fn main() {
    let mut rt = Runtime::new();
    let (asks_in, rows, total) = rt.mutate(|b| {
        let (votes, votes_in) = b.input::<u32>();
        let total = votes.accumulate(b, 0u32, |v, t| t + v);
        let (asks, asks_in) = b.input::<String>();
        let rows = asks
            .with(votes_in)
            .map(|(input, who)| (who, input))
            .share(b);
        (b.anchor(asks_in), b.anchor(rows), b.anchor(total))
    });

    let remote = rt.remote_io();
    let (to_worker, from_listener) = mpsc::channel();
    let handle = remote.clone();
    remote
        .listen(&rows, move |(who, input), at| {
            let _ = to_worker.send((who, handle.anchor(at, input)));
        })
        .keep();
    let worker = thread::spawn(move || {
        let (who, input) = from_listener.recv().expect("a row");
        remote.send(&input, 5);
        who
    });

    rt.pump();
    rt.send(&asks_in, "ada".to_string());
    while !worker.is_finished() {
        thread::yield_now();
    }
    rt.pump();
    let who = worker.join().expect("the worker");
    let total = rt.mutate(move |b| b.sample(b.open(&total)));
    println!(
        "row for {who} anchored by the listener at its At; a worker's send through it made the total {total:?}"
    );
}
