//! Must fail: a token returned from `mutate` rather than anchored.
#![forbid(unsafe_code)]
#![cfg_attr(bough_auto, feature(auto_traits, negative_impls))]
#[path = "api.rs"]
mod bough;
use bough::*;

fn main() {
    let mut rt = Runtime::new();
    let n_in = rt.mutate(|b| b.input::<u32>().1);
    let _ = n_in;
}
