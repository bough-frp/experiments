//! Must build: a type parameter with a bound of its own. `Of<'x>` is
//! `Pinned<T::Of<'x>>`, which must meet `Pinned`'s bounds too, so the derive
//! copies each bound on `T` to `for<'x> T::Of<'x>`; without that the impl
//! fails with "`<T as Rebrand>::Of<'x>: Clone` is not satisfied".
#![forbid(unsafe_code)]
use bough::*;
#[derive(Clone, Rebrand)]
struct Pinned<T: Clone>
where
    T: Default,
{
    value: T,
    count: u32,
}

#[derive(Clone, Default, Rebrand)]
struct Plain {
    n: u32,
}

fn stored<T: Rebrand>() {}

fn main() {
    stored::<Pinned<Plain>>();
    let mut rt = Runtime::new();
    rt.mutate(|b| {
        let (_s, _i) = b.input::<Pinned<Option<Cell<'_, u32>>>>();
    });
}
