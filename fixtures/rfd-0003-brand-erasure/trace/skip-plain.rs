//! Must fail, by choice: a skipped `String`. It holds no token, but it
//! implements `Rebrand`, so the skip changes nothing but what the collector
//! sees. The check can't tell a `String` from a token container by anything
//! but the impls, so it refuses both and says to drop the skip.
#![forbid(unsafe_code)]
use bough::*;

#[derive(Clone, Rebrand, Trace)]
struct Label {
    #[rebrand(skip)]
    text: String,
}

fn main() {}
