# Experiments

Experiments backing Bough's RFDs, in the `rfd` repository's `src/`, and
the research notes that argue from them.

Any code that produces concrete data used to argue an RFD lives here
rather than in a scratch file or a gist. An RFD that cites a measurement
is only as good as a reader's ability to re-run it, and a benchmark that
lived in someone's working tree cannot be re-run at all.

## Layout

This repository is a sub-project of the design work. It is never
published, and it is held to the same checks as anything else, `cargo
fmt` and clippy, because an experiment is still code someone runs, and
not publishing it is no reason to skip that. An experiment that brings a
dependency brings it into this repository; [Retirement](#retirement)
says when it leaves.

> **Sub-project:** one Cargo package, `bough-experiments`, at the root,
> which is also a workspace. A probe is a target of the root package. A
> member crate joins only when the next probe is incompatible with every
> existing one: two versions of a crate that Cargo can't both satisfy, a
> `links` collision, feature unification that would change what a probe
> measures (check `cargo tree -e features` first), or another toolchain.
> A crate on another toolchain is excluded from the workspace and carries
> its own `rust-toolchain.toml`.

The toolchain is pinned exactly in `rust-toolchain.toml`, at the stable
release on the day this was set up, and `Cargo.lock` is committed.
Criterion is 0.8, and gungraun is 0.19.4, to match its runner:
`cargo install gungraun-runner --version 0.19.4 --locked`. The
instruction counts need valgrind.

Each experiment is named after the RFD whose question it informs,
`rfd-NNNN-<slug>`. A probe that doesn't measure performance is a binary:

```text
src/bin/rfd-0001-some-probe.rs
```

```shell
cargo run --release --bin rfd-0001-some-probe
```

A probe that measures performance is two benches, a wall-clock one
under Criterion and an instruction count under gungraun:

```text
benches/rfd-0005-some-probe-wallclock.rs
benches/rfd-0005-some-probe-instructions.rs
```

```shell
cargo bench --bench rfd-0005-some-probe-wallclock
cargo bench --bench rfd-0005-some-probe-instructions
```

The code both benches measure is the probe's module in the library,
`src/rfd_0005_some_probe.rs`, so the two measure the same thing. That
module is the probe's own; it isn't shared.

A probe that must show a program failing to compile can't be a target,
because a target that doesn't build breaks the build. Its cases are
source files under `fixtures/<name>/`, outside every target, and its
binary compiles each one with the pinned `rustc` and prints whether it
built and what the compiler said. Compile time is measured the same way,
by a binary that builds generated code, and it's wall-clock, so it is
run with the benches on an idle machine.

Every wall-clock probe measures its own baseline in the same run. What
a note quotes is the ratio to that baseline, with its interval, never an
absolute time.

`scripts/ratios.py` computes those ratios after a Criterion run. It
compares each `<group>/<variant>/<param>` with `<group>/baseline/<param>`
and bootstraps a 95% interval from Criterion's samples, with a fixed
seed.

Results go in `results/`, one file per run of a target, named
`<target>-<date>.txt`. Each starts with its provenance line, then the
command, then the output as it came:

```text
> rustc VERSION (released DATE) - measured DATE - <target> at experiments@COMMIT - Ryzen 7 2700X, <container OS>
```

`COMMIT` is the commit the target was built from, so the results are
committed after it. Print results in whatever shape the note needs to
quote them. Fixtures shared between experiments, such as a graph builder
two probes both need, get a shared module, and it stays empty until
something is actually shared. Resist putting an experiment's own
scaffolding there.

## What does not go here

Research produces *evidence*, not tests. It isn't asserting that Bough
is correct, and it isn't expected to keep passing. It answers a question
that was open when the RFD was written. Bough itself isn't a dependency
yet: the probes model the engine the RFDs describe, in the few hundred
lines each question needs.

No playground is used. Every experiment builds here, and the ones that
must fail to build are fixtures, as above.

## Retirement

An experiment is maintained while the RFD it serves is still being
argued or built. It leaves the moment that RFD reaches `committed` or
`abandoned`, through one of two exits:

- **Deleted.** It measured internals the RFD replaced. A probe that no
  longer builds against the new code is deleted rather than repaired.
  The note that quotes its numbers cites the commit that produced them,
  and history keeps it.
- **Promoted.** It still answers a live question, which means it stopped
  being research. A measurement worth re-running is a benchmark and moves
  to `bough-bench`. Something asserting a property Bough promises is a
  test and moves to the test suite.

An experiment that brought a dependency takes it out through whichever
exit it leaves by. Deleted means the manifest entry goes in the same
commit. Promoted means the dependency is promoted too, and that is a
decision somebody makes, not a side effect of moving a file.

This repository is a staging area, not an archive. Everything in it has
a scheduled exit, and a healthy one trends toward empty.
