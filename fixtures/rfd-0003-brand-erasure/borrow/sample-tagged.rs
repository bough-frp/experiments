//! Must build and run: views of generic types and of an enum. `Tagged<T>`
//! has a type parameter and no brand, so its view is `TaggedRef<'a, T>` with
//! `value: T::Ref<'a>`: a token at `Cell`, a `ListRef` at `Vec`, `&u32` at
//! `u32`. `Keyed<'g, T>` has both, `KeyedRef<'a, 'g, T>`. `Screen<'g>` is an
//! enum, and its view an enum of the same variants, matched as the value.
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

/// Generic over what is tagged: the view's `tag` is `&String` at every `T`.
fn tag_of<'a, T: Borrow>(t: TaggedRef<'a, T>) -> &'a str {
    t.tag
}

fn screen<'g>(b: &Build<'g>, s: ScreenRef<'_, 'g>) -> String {
    match s {
        ScreenRef::Login { user } => format!("Login {{ user: {:?} }}", b.sample(user)),
        ScreenRef::Chat(x, y) => format!("Chat({:?}, {:?})", b.sample(x), b.sample(y)),
        ScreenRef::Closed => "Closed".to_string(),
    }
}

/// A cell holding `value`: an input never sent, held.
fn held<'g, T: Rebrand + Trace>(b: &mut Build<'g>, value: T) -> Cell<'g, T> {
    let (never, _) = b.input::<T>();
    never.hold(b, value)
}

fn main() {
    let mut rt = Runtime::new();
    let (n_in, cells) = rt.mutate(|b| {
        let (n, n_in) = b.input::<u32>();
        let n = n.share(b);
        let x = n.hold(b, 0);
        let y = n.map(|v| v + 10).hold(b, 0);
        let one = held(
            b,
            Tagged {
                tag: "one".to_string(),
                value: x,
            },
        );
        let many = held(
            b,
            Tagged {
                tag: "many".to_string(),
                value: vec![x, y],
            },
        );
        let plain = held(
            b,
            Tagged {
                tag: "plain".to_string(),
                value: 42u32,
            },
        );
        let nested = held(
            b,
            Tagged {
                tag: "outer".to_string(),
                value: Tagged {
                    tag: "inner".to_string(),
                    value: y,
                },
            },
        );
        let keyed = held(
            b,
            Keyed {
                source: x,
                all: vec![
                    Tagged {
                        tag: "a".to_string(),
                        value: x,
                    },
                    Tagged {
                        tag: "b".to_string(),
                        value: y,
                    },
                ],
                first: Some(Tagged {
                    tag: "a".to_string(),
                    value: x,
                }),
            },
        );
        let screens = held(
            b,
            vec![
                Screen::Login { user: x },
                Screen::Chat(x, y),
                Screen::Closed,
            ],
        );
        (
            b.anchor(n_in),
            b.anchor(((one, many, plain), (nested, keyed, screens))),
        )
    });
    rt.send(&n_in, 2);
    rt.mutate(|b| {
        let ((one, many, plain), (nested, keyed, screens)) = b.open(&cells);

        let one = b.sample_ref(one).expect("live");
        println!("one:     {} = {:?}", tag_of(one), b.sample(one.value));
        let many = b.sample_ref(many).expect("live");
        println!(
            "many:    {} = {:?}",
            tag_of(many),
            many.value.iter().map(|c| b.sample(c)).collect::<Vec<_>>()
        );
        let plain: TaggedRef<'_, u32> = b.sample_ref(plain).expect("live");
        println!("plain:   {} = {}", tag_of(plain), plain.value);
        let nested = b.sample_ref(nested).expect("live");
        println!(
            "nested:  {}.{} = {:?}",
            nested.tag,
            nested.value.tag,
            b.sample(nested.value.value)
        );
        let keyed: KeyedRef<'_, '_, Tagged<Cell<'_, u32>>> = b.sample_ref(keyed).expect("live");
        println!(
            "keyed:   source {:?} all {:?} first {:?}",
            b.sample(keyed.source),
            keyed
                .all
                .iter()
                .map(|t| (t.tag.as_str(), b.sample(t.value)))
                .collect::<Vec<_>>(),
            keyed.first.map(|t| b.sample(t.value)),
        );
        let screens = b.sample_ref(screens).expect("live");
        let shown: Vec<String> = screens.iter().map(|s| screen(b, s)).collect();
        println!("screens: {}", shown.join(", "));
    });
}
