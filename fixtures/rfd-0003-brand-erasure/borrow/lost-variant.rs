//! Must fail, recording what the write view gives up: replacing a field of
//! enum (or struct) type whole. The parent's write view holds the field's
//! own write view, an enum of its variants' field views, which can set the
//! tokens of the current variant but not change it; `&mut S` would assign.
//! Only the root `Place`, and a list's or option's `set`, replace whole.
#![forbid(unsafe_code)]
use bough::*;

#[derive(Clone, Copy, Rebrand)]
enum Screen<'g> {
    Login { user: Cell<'g, u32> },
    Closed,
}

#[derive(Rebrand)]
struct App<'g> {
    screen: Screen<'g>,
    title: String,
}

impl Trace for App<'_> {
    fn trace(&self, tracer: &mut Tracer) {
        if let Screen::Login { user } = self.screen {
            user.trace(tracer);
        }
    }
}

fn main() {
    let mut rt = Runtime::new();
    rt.mutate(|b| {
        let (n, _) = b.input::<u32>();
        let user = n.hold(b, 0);
        let (never, _) = b.input::<App>();
        let app = never.hold(
            b,
            App {
                screen: Screen::Login { user },
                title: "app".to_string(),
            },
        );
        b.update(app, |mut p| {
            let a = p.as_mut();
            *a.screen = Screen::Closed;
        })
        .expect("live");
    });
}
