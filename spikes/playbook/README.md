# Playbook legibility spikes

## Status: non-normative

Nothing in `spikes/` is normative, and nothing here is a commitment.

- This directory is **not** a schema source. `crates/zoomieball-core/src/playbook.rs`
  and the acknowledged decisions in [`../../DESIGN.md`](../../DESIGN.md) and
  [`../../GAME_TICK.md`](../../GAME_TICK.md) are the only schema authorities.
- This directory is **not** a conformance input. No fixture, golden replay, witness,
  or parity suite may read a file from here.
- Nothing here is a **migration target**. There is no reader, no upgrade path, and no
  obligation to keep a spike loadable after the schema moves.
- RON dialects here are **throwaway** and may contradict the shipped schema, including
  its version word, its field names, and its accepted vocabulary. A spike that no
  longer compiles is deleted, not fixed.
- `spikes/` is git-tracked but is not a workspace member and carries no `Cargo.toml`.
  Spikes are data plus shell steps, never a crate.

## Why this exists

The largest open risk in Zoomieball is not arithmetic conformance; it is whether the
playbook language is legible enough to be fun to author and to watch. The first
milestone that would otherwise show a play in motion is M4.

A spike drives the existing tracer's play solver with candidate play vocabulary and
reports what the match looks like, without touching the schema. It is optional reading
for the *Acknowledge graph triggers and verb/target shapes* gate in
[`../../TODO.md`](../../TODO.md): that gate's prerequisite is
[`../../docs/graph-v0-proposal.md`](../../docs/graph-v0-proposal.md) alone, and a spike
neither satisfies a prerequisite nor supplies a boundary test. Nothing here changes the
gate's wording or its verdict.

A spike is judged on legibility, not conformance:

- Does the authored intent read off the resulting motion?
- Does a node change visibly change the swarm, and within how many body ticks?
- Can a vocabulary entry be described in one sentence to someone watching?

Tracer physics, arena values, and constants are all pre-conformance, so a spike answers
none of those in absolute terms and must not be quoted as a performance or physics
result.

## Running a spike against the tracer

The headless runner embeds `assets/default-playbook.ron` with `include_str!` and takes
no playbook path argument, so a spike is run by substituting the asset, running the
tracer, then restoring the tree. From the workspace root:

```sh
cp assets/default-playbook.ron /tmp/default-playbook.ron.bak
cp spikes/playbook/<spike>.ron assets/default-playbook.ron
cargo run --release -p zoomieball-headless -- 10 600 --hashes
cp /tmp/default-playbook.ron.bak assets/default-playbook.ron
```

`git status` must be clean afterwards; a spike that leaves `assets/` modified has
escaped its boundary. A spike RON only exercises vocabulary the checked-in compiler
already accepts. Candidate vocabulary beyond that — the triggers, ball verbs, targets,
and coach edge semantics in [`../../docs/graph-v0-proposal.md`](../../docs/graph-v0-proposal.md)
— needs a local, uncommitted compiler edit to run, and that edit is part of the spike
and is thrown away with it.

Record each spike as a sibling `<spike>.md` next to its RON: the vocabulary under test,
the command run, and what was legible. A spike note is an argument someone may cite
while deciding, never a citation a roadmap bite or a document may anchor on.
