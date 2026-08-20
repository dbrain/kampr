# 0001 — The node runs a VT emulator over streamed frames

- **Status:** Accepted
- **Date:** 2026-08-20
- **Shipped in:** `f1f8860` (`kampr-herdr`, `kampr-term`, `kampr-spike`)
- **Evidence:** probes [#10, #12, #13, #14, #16, #22, #23, #24, #25, #37, #41](../03-probe-log.md)
- **Argues against:** Collie [ADR 0008](https://github.com/AltanS/collie), *"Collie does not run a
  terminal emulator"*

## Context

Kampr exists because of a complaint about Collie — a Herdr web bridge that predates it — which is
usually phrased as *"it just wraps text"*. Before writing a line of Kampr it was worth establishing
whether that was a limitation of Herdr or a decision Collie made. It is a decision, and Collie wrote
it down: its ADR 0008 refuses a terminal emulator anywhere in the system, in the browser or in the
bridge, and pins its client contract to `StyledLine[]`.

That ADR is a good piece of reasoning and most of it still holds. Three of its four arguments are
untouched by anything Kampr measured:

- **The emulation already happened one process upstream.** `pane.read` returns Herdr's rendered
  grid, not a byte stream. An emulator downstream of it re-emulates an already-emulated screen.
- **The bug history it was proposed to fix is semantic, not graphical.** *Which rows are the input
  box* is not more answerable from a better-rendered grid.
- **Its fixture corpus is pinned to `pane.read` bytes.** A second renderer differs in exactly the
  ways byte-faithful captures are sensitive to, so adopting one means re-capturing everything.

The first of those is the one that matters here, and it is where Kampr's position departs. Collie's
argument is about `pane.read`. Kampr does not consume `pane.read` for live content — it consumes
`herdr terminal session observe`, which ADR 0008 explicitly declines to evaluate:

> And `HERDR_API.md` verifies **nothing** about `observe`/`control`: not the frame format, not
> whether cursor state is even in it, not multi-observer semantics, not a version floor.

**That is the fact that changed.** `observe`/`control` were unprobed when Collie decided; they are
now measured, and the answers are better than the proposal Collie turned down assumed:

| Question ADR 0008 could not answer | Measured |
| --- | --- |
| What is in a frame? | Base64 ANSI, cursor address, sync markers, `full` flag (#10, #12) |
| Is cursor state in it? | Yes — every frame ends `ESC[r;cH` + `ESC[?25h/l` (#12) |
| Multi-observer semantics? | Many observers, each at its own geometry, concurrently (#13) |
| Does observing disturb the desk? | No. PTY stayed 36 rows under an observer at 20 (#14) |
| Latency? | p50 27 ms, p90 98 ms locally (#22) |
| Bandwidth under load? | `seq 1 20000` → **3 frames, 1.9 KB.** Herdr coalesces to grid state rather than replaying bytes (#23) |
| Headless? | Yes — with no TUI client ever attached (#24) |
| Fidelity? | A frame-fed emulator reproduces Herdr's own grid **30/30 rows**, cursor on the right cell (#41) |

And one that reverses the "re-emulation is not a fidelity gain" claim outright: `pane.read` **drops
OSC 8 hyperlinks**, and frames **keep them** (#36, #37). The frame path is not a second, noisier view
of the same information. It is strictly richer than the read path.

Two further facts made the streaming side of this cheap and the polling side expensive:

- **There is no output-change event on the socket API.** `pane.output_changed` exists as an event
  kind and the subscription validator rejects it; `pane.updated` stayed silent through three seconds
  of output (#4, #5). Building on the JSON API alone *forces* polling. That is not Collie being
  conservative; it is the only thing the socket offers.
- **Applying a diff frame requires emulator state.** Only the first frame of a stream is `full`
  (#53); the rest are cursor-addressed partial repaints. Something has to hold the grid. The choice
  is not *emulator or no emulator* — it is *where*.

Given that something must emulate, the options were a Kotlin emulator in the client's `commonMain`
or a Rust one in the node. The required subset turned out to be small: Herdr's serialiser emits
absolute cursor addressing, SGR, erase, and the sync/hyperlink markers — no scroll regions, no
relative motion — so `kampr-term` is a thin `vte` consumer rather than a terminal emulator in the
full sense, and it matched Herdr's own grid exactly on the first serious test.

## Decision

**The node consumes `herdr terminal session observe`, runs one VT emulator per pane, and ships
clients a cell grid.**

- **One emulator per pane, not per viewer.** Three devices watching one pane share a grid.
- **The frame stream is the content path; the socket API is the structure path.** Snapshots and
  events describe panes, tabs and workspaces. They never carry screen content.
- **Live content never comes from `pane.read`.** `pane.read` is used for scrollback backfill
  ([ADR 0004](./0004-scrollback-is-stitched-and-a-gap-discards.md)), for the pending-prompt text,
  and to measure a pane's true width — never to draw the live viewport.
- **The emulator stays minimal and stays verified.** `crates/kampr-spike` reconstructs a pane from
  frames alone and diffs it against Herdr's own `pane.read visible`. It is the pipeline's canary and
  it is expected to print `PERFECT MATCH`.

## Consequences

- **Kampr owns a renderer, with everything that implies.** A Herdr change to its frame serialiser
  breaks Kampr in a way it does not break Collie. The mitigation is the spike, the version floor
  (`min_herdr_version = "0.8.2"`, the only version anything here is verified on), and the probe log.
- **Kampr owns a child process per watched pane geometry.** `observe` is a CLI child, not a socket
  method, so the node spawns and supervises processes. Probe #73 records the sharp edge: Herdr's
  `observe` children **outlive a node killed with `SIGTERM`**, reparented to init and still streaming
  into a closed pipe. `kill_on_drop` cannot help, because a signal skips the destructor.
- **The cost of one child per pane × geometry was never measured.** Roadmap P0.6 is still open. The
  registry refcounts watchers so viewers at the same geometry share one stream, which is the design
  the unmeasured cost argues for, but "20 panes and 3 devices" has not been run.
- **A second renderer disagreeing with the first is a real risk, and it is bounded by a test rather
  than by argument.** Collie is right that this is the danger; Kampr's answer is that the
  disagreement is measurable, and is measured.
- **The stated payoff has only partly been collected.** The argument for emulating in the node
  rather than the client was that selection, find and hyperlinks become node features over a cell
  model instead of three client reimplementations. Hyperlinks did: the link table is interned in the
  node and travels on the wire. **Selection did not** — it lives in
  `client/terminal/.../render/Selection.kt`, in the client, where the argument said it would not.
  **Find does not exist at all.** With one client shipping, the cost of that is zero; with a second,
  it is the whole argument. Either collect it or stop claiming it.

## What would justify revisiting

- **Herdr exposing a structured grid over the socket API.** There is already one —
  `herdr-client.sock` carries `FrameData` with `cells`/`fg`/`bg`/`hyperlink` — but it is
  bincode-framed, private and unversioned to third parties, and using it is on the roadmap's
  explicit cut list. If an equivalent were ever published on the versioned socket, the emulator
  becomes dead weight and this ADR is superseded rather than amended.
- **The spike failing against a new Herdr release.** Not a reason to abandon the approach, but the
  moment the version floor argument stops being theoretical, and the moment to decide whether Kampr
  tracks Herdr's serialiser or pins to a version.
- **A measured process cost that the mux cannot absorb.** P0.6. If one `observe` child per pane per
  geometry is not affordable at herd scale, the shape that survives is fewer geometries — one stream
  per pane at its native size, with every viewer rendering the same grid — not fewer emulators.
