//! Must build and run: question 2. RFD 6's chat room in outline, with
//! standard threads and channels in tokio's place. Each user's thread holds
//! a `RemoteIo` clone and the anchored inputs, and sends to them from its
//! own thread; the driver is the main thread, pumping. Ada and Bob join,
//! then each says a line; then Cy joins and says hi in one transaction,
//! which opens both anchored inputs inside the queued closure, so Cy's join
//! and line are simultaneous and the snapshot of members doesn't have Cy.
//!
//! Changes from RFD 6's text: the outbound listener is registered inside
//! the build's `mutate`, since `outbound` can't leave it; the per-user
//! sender is a `Leaf`, since every event type is now `Rebrand`.
#![forbid(unsafe_code)]
#![cfg_attr(bough_auto, feature(auto_traits, negative_impls))]
#[path = "api.rs"]
mod bough;
use bough::*;
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Barrier};
use std::thread;

type User = String;

/// What `#[derive(Clone, Trace, Rebrand)]` writes for a type with no
/// lifetime or type parameter: its own `Of`, viewed in place.
#[derive(Clone, Default)]
struct Members {
    by_user: HashMap<User, Leaf<Sender<String>>>,
}

impl Trace for Members {
    fn trace(&self, tracer: &mut Tracer) {
        self.by_user.trace(tracer);
    }
}

impl Rebrand for Members {
    type Of<'x> = Members;
    fn rebrand<'x>(&self, _: Witness<'x>) -> Members {
        self.clone()
    }
    fn restore<'x>(from: &Members, _: Witness<'x>) -> Members {
        from.clone()
    }
    fn view(from: &Members) -> Option<&Members> {
        Some(from)
    }
}

fn main() {
    let mut runtime = Runtime::new();
    let (joins, messages) = runtime.mutate(|b| {
        let (joins, joins_in) = b.input::<(User, Leaf<Sender<String>>)>();
        let (messages, messages_in) = b.input::<(User, String)>();
        let members = joins.accumulate_mut(b, Members::default(), |(user, sender), m| {
            m.by_user.insert(user, sender);
        });
        let outbound = messages
            .snapshot(members, |(user, line), m| {
                let recipients: Vec<_> = m.by_user.values().cloned().collect();
                (recipients, format!("{user}: {line}"))
            })
            .node(b);
        b.listen(outbound, |(recipients, text)| {
            for sender in recipients {
                let _ = sender.send(text.clone());
            }
        })
        .keep();
        (b.anchor(joins_in), b.anchor(messages_in))
    });

    let remote = runtime.remote_io();
    let spoken = Arc::new(Barrier::new(3));
    let mut users = Vec::new();
    let joined = Arc::new(Barrier::new(2));
    for user in ["ada", "bob"] {
        let (remote, joins, messages) = (remote.clone(), joins.clone(), messages.clone());
        let (joined, spoken) = (joined.clone(), spoken.clone());
        users.push(thread::spawn(move || {
            let (sender, inbox) = mpsc::channel::<String>();
            remote.send(&joins, (user.to_string(), Leaf(sender)));
            joined.wait();
            remote.send(&messages, (user.to_string(), format!("hello from {user}")));
            spoken.wait();
            let heard: Vec<String> = inbox.iter().take(3).collect();
            (user, heard, None::<Receiver<String>>)
        }));
    }
    users.push(thread::spawn(move || {
        let (sender, inbox) = mpsc::channel::<String>();
        spoken.wait();
        remote.transaction(move |m| {
            let (join, say) = (m.open(&joins), m.open(&messages));
            m.send(join, ("cy".to_string(), Leaf(sender)));
            m.send(say, ("cy".to_string(), "hi".to_string()));
        });
        ("cy", Vec::new(), Some(inbox))
    }));

    while !users.iter().all(|user| user.is_finished()) {
        runtime.pump();
        thread::yield_now();
    }
    runtime.pump();
    for user in users {
        let (name, mut heard, inbox) = user.join().expect("the user's thread");
        heard.extend(inbox.iter().flat_map(|inbox| inbox.try_iter()));
        heard.sort();
        println!("{name} heard {heard:?}");
    }
}
