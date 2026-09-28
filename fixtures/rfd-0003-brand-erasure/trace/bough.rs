//! Bough for the `trace` mode, as a crate of its own: `api.rs` here is the
//! main run's `../api.rs` with question 2's reads appended, and the derive is
//! built with its `trace` feature, so `#[derive(Trace)]` is exported next to
//! `#[derive(Rebrand)]` and both check skipped fields. `Rebrand` and `Trace`
//! for `Box` and `HashMap` are `../derive/bough.rs`'s. A separate crate, so
//! that the orphan rules and privacy apply to fixtures as to a user's.
#![forbid(unsafe_code)]

#[path = "api.rs"]
mod runtime;

pub use rfd_0003_rebrand_derive::{Rebrand, Trace};
pub use runtime::*;

use std::collections::HashMap;
use std::hash::Hash;

impl<T: Rebrand> Rebrand for Box<T> {
    type Of<'x> = Box<T::Of<'x>>;
    fn rebrand<'x>(&self, to: Witness<'x>) -> Self::Of<'x> {
        Box::new((**self).rebrand(to))
    }
    fn restore<'x>(from: &Self::Of<'x>, at: Witness<'x>) -> Self {
        Box::new(T::restore(from, at))
    }
}

impl<T: Trace> Trace for Box<T> {
    fn trace(&self, tracer: &mut Tracer) {
        (**self).trace(tracer);
    }
}

/// Keys are brand-free: a key that held a token would hash by its id, and
/// nothing here needs one.
impl<K: Clone + Eq + Hash + 'static, V: Rebrand> Rebrand for HashMap<K, V> {
    type Of<'x> = HashMap<K, V::Of<'x>>;
    fn rebrand<'x>(&self, to: Witness<'x>) -> Self::Of<'x> {
        self.iter()
            .map(|(k, v)| (k.clone(), v.rebrand(to)))
            .collect()
    }
    fn restore<'x>(from: &Self::Of<'x>, at: Witness<'x>) -> Self {
        from.iter()
            .map(|(k, v)| (k.clone(), V::restore(v, at)))
            .collect()
    }
}
