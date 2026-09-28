//! Must build: a closure capturing a foreign value that holds no tokens, a
//! shared function behind a trait object.
#![feature(auto_traits, negative_impls)]
#[path = "api.rs"]
mod bough;
use bough::*;
use std::rc::Rc;

pub fn program() {
    let mut rt = Runtime::new();
    let _ = rt.build(|b| {
        let (n, n_in) = b.input::<u32>();
        let rule: Rc<dyn Fn(u32) -> u32> = Rc::new(|v| v * 3);
        let tripled = n.map(move |v| rule(v)).hold(b, 0);
        (n_in, tripled)
    });
}
