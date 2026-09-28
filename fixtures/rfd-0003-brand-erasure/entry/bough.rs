//! Bough for the `entry` mode, as a crate of its own: `api.rs` here is
//! `../borrow/api.rs` with the entry bounds and the sealed views, each under a
//! `--cfg` (its module doc lists them), and `Rebrand` for `Box` and `HashMap`
//! as `../derive/bough.rs` has it, so the `derive` mode's fixtures build here
//! too. A separate crate, so that a fixture's hand-written impls see only the
//! public API, as a user's would.
#![cfg_attr(any(not(bough_seal_views), bough_core_forbid), forbid(unsafe_code))]
#![cfg_attr(all(bough_seal_views, not(bough_core_forbid)), deny(unsafe_code))]

#[path = "api.rs"]
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
