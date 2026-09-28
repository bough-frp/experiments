//! Does the DFS mark's grey state catch every same-instant cycle RFD 5's
//! per-move upstream walk catches, and when?
//!
//! Runs each case on the model engine in `bough_experiments::rfd_0005_cycle_in_mark`
//! under two designs: RFD 5 as written (plain mark, an upstream walk at
//! every first link and, once all switches have moved, at every move), and
//! the proposal (a grey mark and no walk). Both keep Brent's guard on reads
//! and the pull's in-progress stamp over nodes built during an instant,
//! which RFD 5 already has. For each case it prints what found the cycle
//! and where: which transaction, and which phase of it. Then it runs the
//! same cases with Brent's guard off, to show which ones need it, and a
//! generated family of graphs where a switch move closes a cycle, to count
//! how many transactions later the grey mark finds it, if ever.

use bough_experiments::rfd_0005_cycle_in_mark::{
    By, Design, Engine, Found, Graph, Id, Item, Kind, NONE, Phase, Tx,
};

/// A case: builds its graph and returns the build's samples and the
/// script after it.
struct Case {
    name: &'static str,
    /// Whether the program is legal: every design should accept it.
    legal: bool,
    build: fn(&mut Graph) -> (Vec<Id>, Vec<Item>),
}

fn tx(fires: &[Id], sets: &[(Id, Id)]) -> Item {
    Item::Tx(Tx {
        fires: fires.to_vec(),
        sets: sets.to_vec(),
        ..Tx::default()
    })
}

/// Runs a case, and returns what found its cycle, if anything did, and
/// how many transactions ran.
fn run(case: &Case, design: Design, brent: bool, reverse: bool) -> (Option<Found>, u32) {
    let mut script = Vec::new();
    let mut e = Engine::build(design, brent, |g| {
        let (samples, s) = (case.build)(g);
        script = s;
        samples
    });
    e.reverse_relinks = reverse;
    let total = script.iter().filter(|i| matches!(i, Item::Tx(_))).count() as u32;
    for item in script {
        e.run(item);
    }
    (e.found, total)
}

fn describe(found: Option<Found>, total: u32) -> String {
    match found {
        None => format!("nothing, {total} tx"),
        Some(f) => {
            let phase = match f.phase {
                Phase::Build => "build",
                Phase::Mark => "mark",
                Phase::Evaluate => "evaluate",
                Phase::Construct => "construct",
                Phase::Relink => "relink",
                Phase::Dispatch => "dispatch",
                Phase::Sample => "sample after",
            };
            let by = match f.by {
                By::Walk => "walk",
                By::GreyMark => "grey mark",
                By::ReadGuard => "Brent",
                By::PullGuard => "pull stamp",
                By::Overflow => "OVERFLOW",
            };
            format!("tx{} {phase}, {by}", f.tx)
        }
    }
}

// ---------------------------------------------------------------- the cases

/// F46: two switches reverse a dependency between them in one instant.
/// b follows p = a + 10 and moves to a constant; a moves from x to
/// y = b + 100. Acyclic once both have moved.
fn f46(g: &mut Graph) -> (Vec<Id>, Vec<Item>) {
    let sel = g.input();
    let (to_q, to_y) = (g.stream(&[sel]), g.stream(&[sel]));
    let (x, q) = (g.constant(NONE), g.constant(NONE));
    let a_forward = g.forward();
    let p = g.map_cell(&[a_forward], NONE);
    let b_outer = g.hold(to_q, p);
    let b = g.switch_cell(b_outer);
    let y = g.map_cell(&[b], NONE);
    let a_outer = g.hold(to_y, x);
    let a = g.switch_cell(a_outer);
    g.close(a_forward, a);
    let sets = [(b_outer, q), (a_outer, y)];
    (vec![], vec![tx(&[sel], &sets), tx(&[sel], &sets)])
}

/// F50's shape, `upstream` nodes above the new inner, `downstream` below
/// the switch. With `cycle`, the middle upstream node also depends on the
/// switch's last downstream node, through a stream loop, so the move
/// closes a cycle through half of it. Script: the move, an input that
/// reaches nothing of it, the selector again, the source.
fn f50(g: &mut Graph, upstream: u32, cycle: bool) -> (Vec<Id>, Vec<Item>) {
    let source = g.input();
    let selector = g.input();
    let other = g.input();
    g.stream(&[other]);
    // A stream loop, closed below with a stream of the switch's last
    // downstream node, feeds the middle upstream node when `cycle` is on.
    // Before the move that closes nothing.
    let feedback = g.forward();
    let first = g.stream(&[source]);
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    for i in 1..upstream {
        let other = first + (xorshift(&mut seed) % i as u64) as Id;
        let prev = first + i - 1;
        let mut deps = vec![prev];
        if other != prev {
            deps.push(other);
        }
        if cycle && i == upstream / 2 {
            deps.push(feedback);
        }
        g.stream(&deps);
    }
    let inner = first + upstream - 1;
    let old = g.constant(NONE);
    let select = g.stream(&[selector]);
    let outer = g.hold(select, old);
    let switch = g.switch_cell(outer);
    let mut last = switch;
    for _ in 0..100 {
        last = g.stream(&[last]);
    }
    if cycle {
        let back = g.stream(&[last]);
        g.close(feedback, back);
    }
    let set = [(outer, inner)];
    let script = vec![
        tx(&[selector], &set),
        tx(&[other], &[]),
        tx(&[selector], &set),
        tx(&[source], &[]),
    ];
    (vec![], script)
}

/// R10b: a loop c closed with a switch that moves to a map_cell of c. The
/// move closes c -> m -> switch -> c. Script: the move, an unrelated
/// input, the selector again.
fn r10b_graph(g: &mut Graph) -> (Id, Id, Id, Id, Id) {
    let sel = g.input();
    let other = g.input();
    g.stream(&[other]);
    let to_m = g.stream(&[sel]);
    let a = g.constant(NONE);
    let c = g.forward();
    let m = g.map_cell(&[c], NONE);
    let outer = g.hold(to_m, a);
    let sw = g.switch_cell(outer);
    g.close(c, sw);
    (sel, other, c, m, outer)
}

fn r10b(g: &mut Graph) -> (Vec<Id>, Vec<Item>) {
    let (sel, other, _, m, outer) = r10b_graph(g);
    let set = [(outer, m)];
    (
        vec![],
        vec![tx(&[sel], &set), tx(&[other], &[]), tx(&[sel], &set)],
    )
}

/// R10b and a `Runtime::sample(c)` after the move.
fn r10b_sampled(g: &mut Graph) -> (Vec<Id>, Vec<Item>) {
    let (sel, other, c, m, outer) = r10b_graph(g);
    let set = [(outer, m)];
    let script = vec![
        tx(&[sel], &set),
        Item::Sample(c),
        tx(&[other], &[]),
        tx(&[sel], &set),
    ];
    (vec![], script)
}

/// R10b with a cell listener on c: c steps at the move, and its listener
/// reads it at dispatch.
fn r10b_listened(g: &mut Graph) -> (Vec<Id>, Vec<Item>) {
    let (sel, other, c, m, outer) = r10b_graph(g);
    let set = [(outer, m)];
    let moving = Item::Tx(Tx {
        fires: vec![sel],
        sets: set.to_vec(),
        listen: vec![c],
        ..Tx::default()
    });
    (vec![], vec![moving, tx(&[other], &[]), tx(&[sel], &set)])
}

/// R10: R10b with c's steps, which reads c after the instant at the move,
/// through the switch's new selection, before commit.
fn r10(g: &mut Graph) -> (Vec<Id>, Vec<Item>) {
    let (sel, _, c, m, outer) = r10b_graph(g);
    g.steps(c);
    (vec![], vec![tx(&[sel], &[(outer, m)])])
}

/// F57: a switch of a switch. At the move the middle switch selects a
/// map_cell of a loop closed with the top switch; relink's first pass
/// reads the top's new selection through the middle's and goes around.
fn f57(g: &mut Graph) -> (Vec<Id>, Vec<Item>) {
    let sel = g.input();
    let to_computed = g.stream(&[sel]);
    let one = g.constant(NONE);
    let first = g.constant(one);
    let forward = g.forward();
    let computed = g.map_cell(&[forward], one);
    let outer = g.hold(to_computed, first);
    let middle = g.switch_cell(outer);
    let top = g.switch_cell(middle);
    g.close(forward, top);
    (vec![], vec![tx(&[sel], &[(outer, computed)])])
}

/// F19, F49: a loop closed with a switch a constant points at the loop (or
/// at a map_cell of it), sampled in the closure that built it, before the
/// switch's first link. In a `construct` closure at tx1.
fn f19_closure(through_a_map_cell: bool) -> impl FnOnce(&mut Graph) -> Vec<Id> {
    move |g| {
        let forward = g.forward();
        let selected = if through_a_map_cell {
            g.map_cell(&[forward], NONE)
        } else {
            forward
        };
        let outer = g.constant(selected);
        let switched = g.switch_cell(outer);
        g.close(forward, switched);
        vec![switched]
    }
}

fn construct(fires: Id, f: impl FnOnce(&mut Graph) -> Vec<Id> + 'static) -> Item {
    Item::Tx(Tx {
        fires: vec![fires],
        construct: Some(Box::new(f)),
        ..Tx::default()
    })
}

fn f19_construct(g: &mut Graph) -> (Vec<Id>, Vec<Item>) {
    let go = g.input();
    g.stream(&[go]);
    (vec![], vec![construct(go, f19_closure(false))])
}

fn f19_construct_map(g: &mut Graph) -> (Vec<Id>, Vec<Item>) {
    let go = g.input();
    g.stream(&[go]);
    (vec![], vec![construct(go, f19_closure(true))])
}

fn f19_build(g: &mut Graph) -> (Vec<Id>, Vec<Item>) {
    (f19_closure(true)(g), vec![])
}

/// F56: no sample. A stream loop built first closes with a snapshot of the
/// switch over the closure's own stream, so pulling the loop pulls the
/// snapshot, which reads the switch before its first link.
fn f56(g: &mut Graph) -> (Vec<Id>, Vec<Item>) {
    let go = g.input();
    let go_s = g.stream(&[go]);
    let closure = move |g: &mut Graph| {
        let pulled = g.forward();
        let forward = g.forward();
        let outer = g.constant(forward);
        let switched = g.switch_cell(outer);
        g.close(forward, switched);
        let reads = g.snapshot(go_s, switched);
        g.close(pulled, reads);
        g.hold(pulled, NONE);
        vec![]
    };
    (vec![], vec![construct(go, closure)])
}

/// A switch a closure builds whose first link closes a cycle, with nothing
/// reading it.
fn first_link_construct(g: &mut Graph) -> (Vec<Id>, Vec<Item>) {
    let go = g.input();
    g.stream(&[go]);
    let closure = |g: &mut Graph| {
        let forward = g.forward();
        let next = g.map_cell(&[forward], NONE);
        let outer = g.constant(next);
        let switched = g.switch_cell(outer);
        g.close(forward, switched);
        vec![]
    };
    (vec![], vec![construct(go, closure)])
}

/// The same, in the build.
fn first_link_build(g: &mut Graph) -> (Vec<Id>, Vec<Item>) {
    let forward = g.forward();
    let next = g.map_cell(&[forward], NONE);
    let outer = g.constant(next);
    let switched = g.switch_cell(outer);
    g.close(forward, switched);
    (vec![], vec![])
}

/// A switch_stream that comes to follow a stream computed from its own
/// events. With `merged`, that stream also merges input y, which later
/// fires. Script: x, the move, x, the selector, y.
fn switch_stream_self(g: &mut Graph, merged: bool) -> (Vec<Id>, Vec<Item>) {
    let x = g.input();
    let sel = g.input();
    let y = g.input();
    let outer = g.forward();
    let out = g.switch_stream(outer);
    let derived = if merged {
        g.stream(&[out, y])
    } else {
        g.stream(&[out])
    };
    let to_derived = g.stream(&[sel]);
    let definition = g.hold(to_derived, x);
    g.close(outer, definition);
    let set = [(definition, derived)];
    let script = vec![
        tx(&[x], &[]),
        tx(&[sel], &set),
        tx(&[x], &[]),
        tx(&[sel], &set),
        tx(&[y], &[]),
    ];
    (vec![], script)
}

/// The same, followed from the start: refused at first link in the build.
fn switch_stream_self_build(g: &mut Graph) -> (Vec<Id>, Vec<Item>) {
    let outer = g.forward();
    let out = g.switch_stream(outer);
    let derived = g.stream(&[out]);
    let definition = g.constant(derived);
    g.close(outer, definition);
    (vec![], vec![])
}

/// The legal navigation loop: a switch_stream's own events select its next
/// inner, through its outer, which is not a dependency.
fn navigation(g: &mut Graph) -> (Vec<Id>, Vec<Item>) {
    let a = g.input();
    let b = g.input();
    let outer = g.forward();
    let out = g.switch_stream(outer);
    let nav = g.stream(&[out]);
    let definition = g.hold(nav, a);
    g.close(outer, definition);
    let script = vec![
        tx(&[a], &[(definition, b)]),
        tx(&[b], &[(definition, a)]),
        tx(&[a], &[]),
    ];
    (vec![], script)
}

const CASES: &[Case] = &[
    Case {
        name: "F46 two-switch reversal",
        legal: true,
        build: f46,
    },
    Case {
        name: "F50 10,000 upstream, acyclic",
        legal: true,
        build: |g| f50(g, 10_000, false),
    },
    Case {
        name: "F50 10,000 upstream, move closes cycle",
        legal: false,
        build: |g| f50(g, 10_000, true),
    },
    Case {
        name: "R10b move closes cycle, no reader",
        legal: false,
        build: r10b,
    },
    Case {
        name: "R10b, then Runtime::sample",
        legal: false,
        build: r10b_sampled,
    },
    Case {
        name: "R10b, cell listener on the loop",
        legal: false,
        build: r10b_listened,
    },
    Case {
        name: "R10 steps view reads after instant",
        legal: false,
        build: r10,
    },
    Case {
        name: "F57 relink reads a switch of a switch",
        legal: false,
        build: f57,
    },
    Case {
        name: "F19/F49 construct samples switch",
        legal: false,
        build: f19_construct,
    },
    Case {
        name: "F19/F49 same, through a map_cell",
        legal: false,
        build: f19_construct_map,
    },
    Case {
        name: "F19/F49 same, in the build",
        legal: false,
        build: f19_build,
    },
    Case {
        name: "F56 construct pulls a snapshot",
        legal: false,
        build: f56,
    },
    Case {
        name: "first link closes cycle, construct",
        legal: false,
        build: first_link_construct,
    },
    Case {
        name: "first link closes cycle, build",
        legal: false,
        build: first_link_build,
    },
    Case {
        name: "switch_stream selects own output",
        legal: false,
        build: |g| switch_stream_self(g, false),
    },
    Case {
        name: "same, output merged with input y",
        legal: false,
        build: |g| switch_stream_self(g, true),
    },
    Case {
        name: "switch_stream own output, build",
        legal: false,
        build: switch_stream_self_build,
    },
    Case {
        name: "switch_stream navigation loop",
        legal: true,
        build: navigation,
    },
];

// ------------------------------------------------------ the generated family

fn xorshift(s: &mut u64) -> u64 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    s.wrapping_mul(0x2545_f491_4f6c_dd1d)
}

/// Every node `from` reaches over dependents.
fn downstream(g: &Graph, from: Id) -> Vec<bool> {
    let mut seen = vec![false; g.len() as usize];
    let mut stack = vec![from];
    seen[from as usize] = true;
    while let Some(n) = stack.pop() {
        for &d in g.dependents(n) {
            if !seen[d as usize] {
                seen[d as usize] = true;
                stack.push(d);
            }
        }
    }
    seen
}

const INPUTS: u32 = 4;
const AFTER: u32 = 20;

/// A random acyclic graph with one switch, and a script whose first
/// transaction moves the switch to `X` and whose next `AFTER` each fire one
/// of five inputs, the selector included. With `cyclic`, X is downstream
/// of the switch, so the move closes a cycle; without, it is not.
///
/// Four inputs and two constants are the sources; each of 20 to 200 nodes
/// depends on one or two earlier nodes, and the switch, a third of the way
/// in, is picked as a dependency more often than the rest. Returns `None`
/// when no X fits.
fn generated(g: &mut Graph, seed: u64, stream: bool, cyclic: bool) -> Option<Vec<Item>> {
    let mut s = seed | 1;
    let mut r = |n: u64| xorshift(&mut s) % n;
    let inputs: Vec<Id> = (0..INPUTS).map(|_| g.input()).collect();
    let sel = g.input();
    let mut pool: Vec<Id> = inputs.clone();
    pool.push(g.constant(NONE));
    pool.push(g.constant(NONE));
    let n = 20 + r(181) as u32;
    let mut switch = NONE;
    let mut outer = NONE;
    for i in 0..n {
        if i == n / 3 {
            let old = pool[r(pool.len() as u64) as usize];
            let select = g.stream(&[sel]);
            outer = g.hold(select, old);
            switch = if stream {
                g.switch_stream(outer)
            } else {
                g.switch_cell(outer)
            };
            pool.push(switch);
            continue;
        }
        let pick = |r: &mut dyn FnMut(u64) -> u64| {
            if switch != NONE && r(5) == 0 {
                switch
            } else {
                pool[r(pool.len() as u64) as usize]
            }
        };
        let a = pick(&mut r);
        let b = pick(&mut r);
        let node = if r(2) == 0 || a == b {
            g.stream(&[a])
        } else {
            g.stream(&[a, b])
        };
        pool.push(node);
    }
    let below = downstream(g, switch);
    let fits: Vec<Id> = pool
        .iter()
        .copied()
        .filter(|&x| x != switch && below[x as usize] == cyclic)
        .collect();
    if fits.is_empty() {
        return None;
    }
    let target = fits[r(fits.len() as u64) as usize];
    let set = [(outer, target)];
    let mut script = vec![tx(&[sel], &set)];
    for _ in 0..AFTER {
        let k = r(INPUTS as u64 + 1) as usize;
        if k == INPUTS as usize {
            script.push(tx(&[sel], &set));
        } else {
            script.push(tx(&[inputs[k]], &[]));
        }
    }
    Some(script)
}

#[derive(Default)]
struct Tally {
    trials: u32,
    walk_at_move: u32,
    walk_other: u32,
    grey_next: u32,
    grey_2_to_5: u32,
    grey_6_on: u32,
    grey_never: u32,
    grey_never_unreachable: u32,
    grey_other: u32,
    agrees_with_reach: u32,
}

/// Runs one trial under both designs and adds it to the tally.
fn trial(t: &mut Tally, seed: u64, stream: bool, cyclic: bool) -> bool {
    let mut script = None;
    let mut walk = Engine::build(Design::Walk, true, |g| {
        script = generated(g, seed, stream, cyclic);
        vec![]
    });
    let Some(script) = script else {
        return false;
    };
    for item in script {
        walk.run(item);
    }
    let mut fired = Vec::new();
    let mut script = None;
    let mut grey = Engine::build(Design::Grey, true, |g| {
        script = generated(g, seed, stream, cyclic);
        vec![]
    });
    for item in script.expect("the same seed builds the same graph") {
        if let Item::Tx(tx) = &item {
            fired.push(tx.fires[0]);
        }
        grey.run(item);
    }
    t.trials += 1;
    match walk.found {
        Some(Found {
            tx: 1,
            phase: Phase::Relink,
            by: By::Walk,
        }) if cyclic => t.walk_at_move += 1,
        None if !cyclic => {}
        _ => t.walk_other += 1,
    }
    // Which transactions' inputs reach the switch after the move, over
    // dependents. Every cycle the move closes goes through the switch.
    let switch = (0..grey.g.len())
        .find(|&n| matches!(grey.g.kind(n), Kind::SwitchCell | Kind::SwitchStream))
        .expect("a switch");
    let reaches: Vec<bool> = fired
        .iter()
        .map(|&i| downstream(&grey.g, i)[switch as usize])
        .collect();
    let expected = if cyclic {
        (1..fired.len()).find(|&k| reaches[k]).map(|k| k as u32 + 1)
    } else {
        None
    };
    let any_input = (0..grey.g.len())
        .filter(|&n| grey.g.kind(n) == Kind::Input)
        .any(|i| downstream(&grey.g, i)[switch as usize]);
    match grey.found {
        None if cyclic => {
            t.grey_never += 1;
            if !any_input {
                t.grey_never_unreachable += 1;
            }
        }
        None => {}
        Some(Found {
            tx,
            phase: Phase::Mark,
            by: By::GreyMark,
        }) if cyclic => match tx - 1 {
            1 => t.grey_next += 1,
            2..=5 => t.grey_2_to_5 += 1,
            _ => t.grey_6_on += 1,
        },
        Some(_) => t.grey_other += 1,
    }
    if grey.found.map(|f| f.tx) == expected {
        t.agrees_with_reach += 1;
    }
    true
}

fn main() {
    println!("cycle-in-mark: the grey mark against RFD 5's per-move upstream walk");
    println!();
    println!("Per case, what found the cycle: transaction, phase, mechanism. Both designs");
    println!("keep Brent's guard on reads and the pull's in-progress stamp on nodes built");
    println!("during an instant; `walk` also walks upstream at every first link and move,");
    println!("`grey` marks with a grey state instead.");
    println!();
    println!(
        "{:<40} {:<5} {:<24} {:<25} without Brent (walk / grey)",
        "case", "legal", "walk (RFD 5)", "grey mark"
    );
    // For each illegal case the walk itself finds: what finds it in the
    // grey design, and whether at the same transaction and phase.
    let mut walk_found = 0;
    let mut by_grey_mark = Vec::new();
    let mut elsewhere = Vec::new();
    let mut never = Vec::new();
    let mut same_point = 0;
    let mut brent_both = 0;
    let mut illegal = 0;
    for case in CASES {
        let (w, total) = run(case, Design::Walk, true, false);
        let (gr, _) = run(case, Design::Grey, true, false);
        let (wn, _) = run(case, Design::Walk, false, false);
        let (gn, _) = run(case, Design::Grey, false, false);
        let no_brent = if wn == gn {
            describe(wn, total)
        } else {
            format!("{} / {}", describe(wn, total), describe(gn, total))
        };
        println!(
            "{:<40} {:<5} {:<24} {:<25} {}",
            case.name,
            if case.legal { "yes" } else { "no" },
            describe(w, total),
            describe(gr, total),
            no_brent
        );
        if case.legal {
            continue;
        }
        illegal += 1;
        let point = |f: Option<Found>| f.map(|f| (f.tx, f.phase));
        match (w, gr) {
            (Some(w @ Found { by: By::Walk, .. }), g) => {
                walk_found += 1;
                if point(Some(w)) == point(g) {
                    same_point += 1;
                }
                match g {
                    Some(Found {
                        by: By::GreyMark, ..
                    }) => by_grey_mark.push(case.name),
                    Some(_) => elsewhere.push(case.name),
                    None => never.push(case.name),
                }
            }
            (
                Some(Found {
                    by: By::ReadGuard, ..
                }),
                _,
            ) if point(w) == point(gr) => brent_both += 1,
            _ => {}
        }
    }
    println!();
    for reverse in [false, true] {
        let order = if reverse { "a before b" } else { "b before a" };
        let each = run(&CASES[0], Design::WalkEachMove, true, reverse).0;
        let walk = run(&CASES[0], Design::Walk, true, reverse).0;
        let grey = run(&CASES[0], Design::Grey, true, reverse).0;
        println!(
            "F46, relink queue {order}: walk after all moves {}, grey {}, walk after each move {}",
            describe(walk, 2),
            describe(grey, 2),
            describe(each, 2),
        );
    }
    let (walked, ordered) = f50_counts();
    println!(
        "F50 move: the walk visits {walked} nodes; the next mark from the source orders {ordered}"
    );
    println!();
    println!(
        "Generated family: a random DAG of 20 to 200 nodes over {INPUTS} inputs, a selector and"
    );
    println!("two constants, one switch; tx1 moves it, then {AFTER} transactions each fire one of");
    println!("the five inputs at random. Cyclic: the new inner is downstream of the switch.");
    println!();
    println!(
        "{:<13} {:<8} {:>6} {:>9} {:>6} {:>8} {:>8} {:>9} {:>6} {:>13} {:>11}",
        "switch",
        "move",
        "trials",
        "walk@tx1",
        "walk?",
        "grey+1",
        "grey+2-5",
        "grey+6-20",
        "never",
        "no input in",
        "grey=reach"
    );
    for stream in [false, true] {
        for cyclic in [true, false] {
            let mut t = Tally::default();
            let mut seed = 1u64;
            while t.trials < 2000 {
                seed += 1;
                trial(
                    &mut t,
                    seed.wrapping_mul(0x9e37_79b9_7f4a_7c15),
                    stream,
                    cyclic,
                );
            }
            println!(
                "{:<13} {:<8} {:>6} {:>9} {:>6} {:>8} {:>8} {:>9} {:>6} {:>13} {:>11}",
                if stream {
                    "switch_stream"
                } else {
                    "switch_cell"
                },
                if cyclic { "cycle" } else { "acyclic" },
                t.trials,
                t.walk_at_move,
                t.walk_other,
                t.grey_next,
                t.grey_2_to_5,
                t.grey_6_on,
                t.grey_never,
                t.grey_never_unreachable,
                format!("{}/{}", t.agrees_with_reach, t.trials),
            );
            assert_eq!(t.grey_other, 0, "the grey design found something else");
        }
    }
    println!();
    println!("walk?: trials where the walk design found something other than its expected");
    println!("result (a cycle at tx1 relink, or nothing). grey+k: found by the grey mark k");
    println!("transactions after the move. never: not found in {AFTER}. no input in: of those,");
    println!("no input reaches the cycle at all. grey=reach: the grey mark fired exactly at the");
    println!("first transaction whose input reaches the switch.");
    println!();
    println!(
        "Verdict: of {illegal} illegal cases, the walk is what finds {walk_found}; in the grey \
         design the grey mark finds {} of those, all transactions later than the walk ({}), {} \
         are found by another guard ({}), and {} never ({}). {same_point} of the {walk_found} are \
         found at the same transaction and phase. The other {brent_both} are found by Brent \
         before either check runs, in both designs, and overflow without it.",
        by_grey_mark.len(),
        by_grey_mark.join("; "),
        elsewhere.len(),
        elsewhere.join("; "),
        never.len(),
        never.join("; "),
    );
}

/// F50's acyclic move under RFD 5: how many nodes its walk visits, and how
/// many the next transaction's mark orders from the source.
fn f50_counts() -> (u32, usize) {
    let mut script = Vec::new();
    let mut e = Engine::build(Design::Walk, true, |g| {
        script = f50(g, 10_000, false).1;
        vec![]
    });
    let mut script = script.into_iter();
    e.run(script.next().expect("the move"));
    assert!(e.found.is_none());
    let walked = e.g.walked;
    let source = 0;
    e.g.mark::<false>(&[source])
        .expect("the plain mark reports nothing");
    (walked, e.g.order.len())
}
