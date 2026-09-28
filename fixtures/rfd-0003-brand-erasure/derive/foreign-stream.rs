//! Must fail: a stream whose event is another crate's type, unwrapped.
#![forbid(unsafe_code)]
use bough::*;
use foreign::Celsius;

fn main() {
    let mut rt = Runtime::new();
    rt.mutate(|b| {
        let (_t, _t_in) = b.input::<Celsius>();
    });
}
