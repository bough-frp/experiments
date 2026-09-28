//! Must fail: `view` for a generic type by an associated const,
//! `BRAND_FREE`, written per type and combined over the parameters. The
//! const is a value: it tells the program `T::Of<'static>` is `T`, not the
//! type checker, so `&Tagged<T::Of<'static>>` is still not `&Tagged<T>`.
//! Only a cast would convert it, and a cast is `unsafe`.
#![forbid(unsafe_code)]
use bough::*;

trait BrandFree {
    const BRAND_FREE: bool;
}

impl BrandFree for u32 {
    const BRAND_FREE: bool = true;
}

impl<'g, A> BrandFree for Cell<'g, A> {
    const BRAND_FREE: bool = false;
}

#[derive(Clone)]
struct Tagged<T> {
    tag: String,
    value: T,
}

impl<T: Rebrand + BrandFree> Rebrand for Tagged<T> {
    type Of<'x> = Tagged<T::Of<'x>>;
    fn rebrand<'x>(&self, to: Witness<'x>) -> Self::Of<'x> {
        Tagged {
            tag: self.tag.clone(),
            value: self.value.rebrand(to),
        }
    }
    fn restore<'x>(from: &Self::Of<'x>, at: Witness<'x>) -> Self {
        Tagged {
            tag: from.tag.clone(),
            value: T::restore(&from.value, at),
        }
    }
    fn view<'a>(from: &'a Self::Of<'static>) -> Option<&'a Self> {
        if T::BRAND_FREE { Some(from) } else { None }
    }
}

fn main() {}
