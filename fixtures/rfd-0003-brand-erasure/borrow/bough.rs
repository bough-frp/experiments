//! Bough for the `borrow` mode, as a crate of its own: `api.rs` here is the
//! main run's `../api.rs` with the borrowed views appended, and the derive is
//! built with its `views` feature, so `#[derive(Rebrand)]` writes `Borrow`
//! and `BorrowMut` too. A separate crate, so that a fixture's hand-written
//! impls see only the public API, as a user's would.
#![forbid(unsafe_code)]

#[path = "api.rs"]
mod runtime;

pub use rfd_0003_rebrand_derive::Rebrand;
pub use runtime::*;
