//! Must build and run: an enum of token-carrying variants, derived, with a
//! named, a tuple and a unit variant. Each variant crosses `anchor` and
//! `open`, and a cell holding the enum switches variant on an event.
#![forbid(unsafe_code)]
use bough::*;

#[derive(Clone, Copy, Rebrand)]
enum Screen<'g> {
    Login { user: Cell<'g, u32> },
    Chat(Cell<'g, u32>, Cell<'g, u32>),
    Closed,
}

impl Trace for Screen<'_> {
    fn trace(&self, tracer: &mut Tracer) {
        match self {
            Screen::Login { user } => user.trace(tracer),
            Screen::Chat(a, b) => {
                a.trace(tracer);
                b.trace(tracer);
            }
            Screen::Closed => {}
        }
    }
}

fn show<'g>(b: &Build<'g>, s: Screen<'g>) -> String {
    match s {
        Screen::Login { user } => format!("Login {{ user: {:?} }}", b.sample(user)),
        Screen::Chat(x, y) => format!("Chat({:?}, {:?})", b.sample(x), b.sample(y)),
        Screen::Closed => "Closed".to_string(),
    }
}

fn main() {
    let mut rt = Runtime::new();
    let (n_in, screens, current) = rt.mutate(|b| {
        let (n, n_in) = b.input::<u32>();
        let n = n.share(b);
        let user = n.hold(b, 1);
        let other = n.map(|v| v + 10).hold(b, 11);
        let screens = vec![
            Screen::Login { user },
            Screen::Chat(user, other),
            Screen::Closed,
        ];
        let chat = Screen::Chat(user, other);
        // The held screen becomes a chat on the first event.
        let current = n.map_to(chat).hold(b, Screen::Login { user });
        (b.anchor(n_in), b.anchor(screens), b.anchor(current))
    });
    rt.mutate(|b| {
        let current = b.open(&current);
        println!("before:  current {}", show(b, b.sample(current).expect("live")));
    });
    rt.send(&n_in, 5);
    rt.mutate(|b| {
        let screens = b.open(&screens);
        let shown: Vec<_> = screens.iter().map(|s| show(b, *s)).collect();
        println!("opened:  {}", shown.join(", "));
        let current = b.open(&current);
        println!("after:   current {}", show(b, b.sample(current).expect("live")));
    });
}
