//! Builds, and shouldn't: `stash-borrow-unsafe.rs` in a crate that doesn't
//! forbid `unsafe_code`. The seal asks the author to say `unsafe`; it can't
//! stop one who does.
include!("stash-borrow-body.rs");
