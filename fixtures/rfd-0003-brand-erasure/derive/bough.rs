//! Bough, as a crate of its own: the runtime of `../api.rs`, unchanged, with
//! the derive re-exported next to its trait and `Rebrand` for the two std
//! containers `api.rs` lacks. A separate crate, so that the orphan rules
//! apply to fixtures as they would to a user of Bough.
#![forbid(unsafe_code)]

#[path = "../api.rs"]
mod runtime;

pub use rfd_0003_rebrand_derive::Rebrand;
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
        self.iter().map(|(k, v)| (k.clone(), v.rebrand(to))).collect()
    }
    fn restore<'x>(from: &Self::Of<'x>, at: Witness<'x>) -> Self {
        from.iter()
            .map(|(k, v)| (k.clone(), V::restore(v, at)))
            .collect()
    }
}
