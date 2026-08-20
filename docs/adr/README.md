# Architecture decision records

Decisions with a **blast radius wider than the diff that made them** — the ones a future
contributor (or a future agent) would otherwise re-derive from scratch, or quietly reverse because
the reasoning lived only in a commit message.

One file per decision, numbered in the order they were accepted:

```
docs/adr/NNNN-kebab-case-title.md
```

Format is [Michael Nygard's](https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions):
**Context** (the forces, including what was actually measured), **Decision** (what we do, in the
imperative), **Consequences** (what this costs), and **What would justify revisiting** — that last
section is the difference between an ADR and a tombstone.

## The house rule about evidence

Kampr's decisions rest on measurements rather than opinions, because the system they are about —
Herdr — is undocumented at the level Kampr needs. **Every claim about Herdr in an ADR carries a
probe number** pointing at [`../03-probe-log.md`](../03-probe-log.md), which traces it to the
command that produced it. A claim with no probe number behind it is a claim to be checked, not
stated. Two of these ADRs exist specifically because a probe overturned a position that was
reasonable before it was measured.

## When to write one

Write an ADR when a decision **closes off an option someone will reasonably propose again**. The
signal is that you find yourself explaining *why not* rather than *how*.

- ✅ "The node runs a VT emulator" — the project Kampr forked its thinking from refuses to, in
  writing, and that refusal is the first thing a reader will find
- ✅ "Kampr never resizes a pane" — the constraint behind a dozen UI compromises, and invisible in
  any single file
- ❌ "Rust and axum on the server" — that is just what the repo is; the findings document covers it
- ❌ Anything already legible from the code, a test name, or a commit message

**The bar is high, and it is meant to be.** These are for the handful of decisions that shape the
system, not a record of work done. If a directory of ADRs reads like a changelog it has stopped
being useful. Before adding one, both of these must be true:

1. **Someone has actually argued for the other road, or demonstrably will.** For most of the entries
   below, the other road is what a comparable project shipped.
2. **The argument has nowhere better to live.** If it fits at the line that would change, put it
   there — whoever reopens the question is reading that code, not this directory. Several
   candidates were turned down on that basis and became file-header comments instead: the interlock
   on scrollback reads, the bracketed-paste framing, the `[[build]]`-downloads-a-binary rule.

A superseded ADR is never deleted or edited into agreement with the present. Mark it
`Superseded by NNNN` and write the new one — the wrong turn is the useful part.

## Relationship to the other docs

Nothing here restates what lives elsewhere; the point is the *reasoning*, once.

| Where | What belongs there |
| --- | --- |
| [`../../ARCHITECTURE.md`](../../ARCHITECTURE.md) | How the system is **built**, as it stands today |
| [`../../README.md`](../../README.md) | How an operator **runs** it |
| [`../01-implementation-findings.md`](../01-implementation-findings.md) | What Herdr **exposes**, and what that makes possible |
| [`../03-probe-log.md`](../03-probe-log.md) | The **evidence** — every claim traced to a command |
| [`../04-wire-protocol.md`](../04-wire-protocol.md) | The node ↔ client **contract** |
| [`../08-threat-model.md`](../08-threat-model.md) | What Kampr **defends**, and what it does not |
| `docs/adr/` | Why a road **wasn't** taken |

## Index

| # | Decision | Status |
| --- | --- | --- |
| [0001](./0001-the-node-runs-a-vt-emulator.md) | The node runs a VT emulator over streamed frames | Accepted |
| [0002](./0002-kampr-never-resizes-a-pane.md) | Kampr never resizes a pane | Accepted |
| [0003](./0003-the-client-contract-is-a-cell-grid.md) | The client contract is a cell grid, not ANSI | Accepted |
| [0004](./0004-scrollback-is-stitched-and-a-gap-discards.md) | Scrollback is stitched from `pane.read`, and a gap discards rather than splices | Accepted |
| [0005](./0005-structure-comes-from-the-transcript.md) | Structure comes from the transcript, never from the grid | Accepted |
| [0006](./0006-auth-is-in-the-node.md) | Auth is in the node, and the origin dictates the ladder | Accepted |
| [0007](./0007-peers-dial-outbound-to-a-hub.md) | Peers dial outbound to a hub | Accepted |
| [0008](./0008-two-render-modes-not-a-glyph-atlas.md) | Two render modes, and never a hand-rolled glyph atlas | Accepted |
| [0009](./0009-the-terminal-keeps-its-own-ground.md) | The terminal keeps its own dark ground, and only its 16 slots are themed | Accepted |
