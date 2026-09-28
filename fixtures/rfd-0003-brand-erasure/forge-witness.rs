//! Must fail: user code makes a `Witness<'static>` to rebrand a token to
//! `'static`, which a graph closure could then capture.
#![forbid(unsafe_code)]
#![cfg_attr(bough_auto, feature(auto_traits, negative_impls))]
#[path = "api.rs"]
mod bough;
use bough::*;

fn main() {
    let mut rt = Runtime::new();
    let _ = rt.mutate(|b| {
        let (n, _) = b.input::<u32>();
        let n = n.share(b);
        let w: Witness<'static> = Witness {
            brand: std::marker::PhantomData,
        };
        let kept: Shared<'static, u32> = n.rebrand(w);
        let _ = kept;
    });
}
