//! Must build and run: write views of generic types and of an enum.
//! `Tagged<T>`'s is `TaggedMut<'a, T>` with `value: T::Mut<'a>`: a `ListMut`
//! at `Vec<Cell>`, a `Place` at a token. `Keyed<'g, T>` has both a brand and
//! a parameter. `Screen<'g>`'s is an enum of the variants' field views: a
//! variant's tokens can be set in place, but the variant can't be changed
//! through it, only by replacing the whole value through a `Place`.
#![forbid(unsafe_code)]
use bough::*;

#[derive(Rebrand)]
struct Tagged<T> {
    tag: String,
    value: T,
}

#[derive(Rebrand)]
struct Keyed<'g, T> {
    source: Cell<'g, u32>,
    all: Vec<T>,
    first: Option<T>,
}

#[derive(Clone, Copy, Rebrand)]
enum Screen<'g> {
    Login { user: Cell<'g, u32> },
    Chat(Cell<'g, u32>, Cell<'g, u32>),
    Closed,
}

impl<T: Trace> Trace for Tagged<T> {
    fn trace(&self, tracer: &mut Tracer) {
        self.value.trace(tracer);
    }
}

impl<T: Trace> Trace for Keyed<'_, T> {
    fn trace(&self, tracer: &mut Tracer) {
        self.source.trace(tracer);
        self.all.trace(tracer);
        self.first.trace(tracer);
    }
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

fn held<'g, T: Rebrand + Trace>(b: &mut Build<'g>, value: T) -> Cell<'g, T> {
    let (never, _) = b.input::<T>();
    never.hold(b, value)
}

fn screen<'g>(b: &Build<'g>, s: ScreenRef<'_, 'g>) -> String {
    match s {
        ScreenRef::Login { user } => format!("Login({:?})", b.sample(user)),
        ScreenRef::Chat(x, y) => format!("Chat({:?}, {:?})", b.sample(x), b.sample(y)),
        ScreenRef::Closed => "Closed".to_string(),
    }
}

/// Generic over what is tagged: the write view's `tag` is `&mut String` at
/// every `T`.
fn retag<T: BorrowMut>(t: TaggedMut<'_, T>, to: &str) {
    *t.tag = to.to_string();
}

fn main() {
    let mut rt = Runtime::new();
    let (n_in, cells, state) = rt.mutate(|b| {
        let (n, n_in) = b.input::<u32>();
        let n = n.share(b);
        let cells: Vec<Cell<u32>> = (1..=3).map(|k| n.map(move |v| v * k).hold(b, 0)).collect();
        let (x, y) = (cells[0], cells[1]);
        let many = held(
            b,
            Tagged {
                tag: "many".to_string(),
                value: vec![x, y],
            },
        );
        let one = held(
            b,
            Tagged {
                tag: "one".to_string(),
                value: x,
            },
        );
        let keyed = held(
            b,
            Keyed {
                source: x,
                all: vec![Tagged {
                    tag: "a".to_string(),
                    value: x,
                }],
                first: None,
            },
        );
        let screens = held(b, vec![Screen::Login { user: x }, Screen::Chat(x, y)]);
        (
            b.anchor(n_in),
            b.anchor(cells),
            b.anchor(((many, one), keyed, screens)),
        )
    });
    rt.send(&n_in, 1);
    rt.mutate(|b| {
        let cells = b.open(&cells);
        let ((many, one), keyed, screens) = b.open(&state);
        let z = cells[2];

        b.update(many, |mut p| {
            let mut t = p.as_mut();
            t.value.push(z);
            t.value.sort_by_key(|c| std::cmp::Reverse(c.index()));
            retag(t, "many, sorted");
        })
        .expect("live");
        b.update(one, |p| p.into_mut().value.set(z)).expect("live");
        b.update(keyed, |mut p| {
            let k = p.as_mut();
            let mut all = k.all;
            all.push(Tagged {
                tag: "b".to_string(),
                value: z,
            });
            if let Some(t) = all.get_mut(0) {
                retag(t, "a2");
            }
            let mut first = k.first;
            first.set(Some(Tagged {
                tag: "f".to_string(),
                value: z,
            }));
        })
        .expect("live");
        b.update(screens, |mut p| {
            let mut list = p.as_mut();
            for s in list.iter_mut() {
                // The variant's tokens, in place.
                match s {
                    ScreenMut::Login { mut user } => user.set(z),
                    ScreenMut::Chat(_, mut to) => to.set(z),
                    ScreenMut::Closed => {}
                }
            }
            // A variant change replaces the element whole.
            list.set(0, Screen::Closed);
        })
        .expect("live");

        let many = b.sample_ref(many).expect("live");
        println!(
            "many:    {} {:?}",
            many.tag,
            many.value.iter().map(|c| b.sample(c)).collect::<Vec<_>>()
        );
        let one = b.sample_ref(one).expect("live");
        println!("one:     {} {:?}", one.tag, b.sample(one.value));
        let keyed = b.sample_ref(keyed).expect("live");
        println!(
            "keyed:   all {:?} first {:?}",
            keyed
                .all
                .iter()
                .map(|t| (t.tag.as_str(), b.sample(t.value)))
                .collect::<Vec<_>>(),
            keyed.first.map(|t| (t.tag.as_str(), b.sample(t.value))),
        );
        let screens = b.sample_ref(screens).expect("live");
        let shown: Vec<String> = screens.iter().map(|s| screen(b, s)).collect();
        println!("screens: {}", shown.join(", "));
    });
}
