//! Must fail: `../borrow/stash-in-borrow.rs` against the sealed views, its
//! stashing `Borrow` impl marked `unsafe` as they require, in a crate that
//! forbids `unsafe_code`.
#![forbid(unsafe_code)]
include!("stash-borrow-body.rs");
