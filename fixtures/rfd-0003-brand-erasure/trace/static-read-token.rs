//! Must fail: `listen_cell_static` on a generic type holding a token. The
//! type has the brand of the `mutate` in it, so it isn't `'static`.
#![forbid(unsafe_code)]
use bough::*;

#[derive(Rebrand, Trace)]
struct Tagged<T> {
    tag: String,
    value: T,
}

fn main() {
    let mut rt = Runtime::new();
    rt.mutate(|b| {
        let (n, _) = b.input::<u32>();
        let n = n.share(b);
        let cell = n.hold(b, 0);
        let held = n
            .map_to(Tagged {
                tag: "cell".to_string(),
                value: cell,
            })
            .hold(
                b,
                Tagged {
                    tag: "cell".to_string(),
                    value: cell,
                },
            );
        b.listen_cell_static(held, |t| println!("{}", t.tag)).keep();
    });
}
