//! Must build: RFD 4's screens example with the screen's input handed to I/O
//! code as a token, which a listener anchors through a `RemoteIo` (RFD 6: a
//! listener handed a row anchors it through a handle). This is how the
//! example is written when graph code can't anchor, as in design `split`.
#![forbid(unsafe_code)]
#![cfg_attr(bough_auto, feature(auto_traits, negative_impls))]
#[path = "api.rs"]
mod bough;
use bough::*;
use std::sync::{Arc, Mutex};

fn main() {
    let mut rt = Runtime::new();
    let remote = rt.remote_io();
    let kept: Arc<Mutex<Vec<Anchored<Input<'static, u32>>>>> = Arc::default();
    let to_io = kept.clone();
    let _ = rt.mutate(move |b| {
        let (opens, opens_in) = b.input::<u32>();
        let made = opens.construct(b, |b, id| {
            let (keys, keys_in) = b.input::<u32>();
            let screen = keys.map(move |key| id * 100 + key).node(b);
            (screen, keys_in)
        });
        let (screens, inputs) = made.unzip(b);
        let idle = b.input::<u32>().0;
        let shown = screens.hold(b, idle).switch_stream(b).share(b);
        b.listen(inputs, move |keys_in| {
            to_io.lock().unwrap().push(remote.anchor(keys_in));
        })
        .keep();
        b.anchor((opens_in, shown))
    });
}
