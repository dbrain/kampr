# 0005 — Structure comes from the transcript, never from the grid

- **Status:** Accepted
- **Date:** 2026-08-20
- **Shipped in:** `e9158ca` (adapters), `a30f092` (the block contract)
- **Evidence:** probes [#38, #39, #42, #43, #44, #45, #30, #72](../03-probe-log.md)

## Context

The complaint Kampr was built to answer is that a phone shows a herd of agents as wrapped terminal
text — a markdown table arriving as box-drawing characters, a diff arriving as coloured rows, a
permission prompt arriving as something you have to transcribe. Fixing the *rendering* of that text
is the obvious move, and it is a dead end. Collie's ADR 0008 states the reason in one line, and it is
correct:

> A TUI paints cells; it does not paint structure.

Both of Herdr's content paths — `pane.read` and `terminal.frame` — are downstream of a renderer. By
the time either exists, the markdown table has *become* box-drawing characters. You cannot recover
it. Attempting to means a heuristic re-parser that will be wrong in ways that are hard to notice,
over content an attacker can influence.

[ADR 0001](./0001-the-node-runs-a-vt-emulator.md) does not change this. A pixel-perfect grid is
still a grid. Emulating better does not recover information that was destroyed upstream.

But there is a second source, and it has the original bytes. Every agent harness Kampr targets keeps
its own transcript on disk in its own format: `~/.claude/projects/<slug>/<uuid>.jsonl` parses to 676
assistant records on this machine whose text is **literal markdown** (#39); Codex writes
`~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`. Nothing has been rendered yet.

This also solves a problem [ADR 0004](./0004-scrollback-is-stitched-and-a-gap-discards.md) cannot —
though not for the reason first written here. That reason was *agent panes run on the alternate
screen, so there is no scrollback ring to read (#30)*, and it was an extrapolation from a synthetic
alt screen rather than a measurement of an agent: a live `codex` reads back 402 rows of ring and a
live `claude` 384 (#231). The transcript is a better history anyway, and for the reason that
survived — it is whole-session, structured and searchable, where a ring is a fixed number of rows
whatever is in it.

Two probes shaped the design more than any argument did:

**Claude does not write a pending tool request to its transcript before you approve it** (#42). A
session held at a permission prompt left the JSONL frozen at 15 680 bytes — the user record only —
for **4 m 20 s**. Nineteen seconds after answering, it jumped to 20 469 bytes carrying *both* the
`tool_use` and its `tool_result`. So for the harness Kampr targets first, the transcript cannot be
the source of the question.

**Codex is the opposite** (#43): the `custom_tool_call` was present 6.2 s in with the prompt still on
screen and the output absent, so an unmatched tool call is Codex's pending signal.

The design had assumed the transcript would answer this for both. It does not, and it is the harness
that will not.

## Decision

**Every pane has two views over the same session. The terminal view comes from the frame stream; the
conversation view comes from the harness's own transcript file. Neither is derived from the other,
and structure is never inferred from a rendered grid.**

- **The conversation view is offered wherever the node has an adapter, and it is the default only
  where the terminal is unusable.** ~~It is the default for an agent pane that has an adapter.~~
  Superseded on 2026-08-31, on the operator's reading of the shipped clients: *"on mobile it
  probably makes sense for conversation to be default — as terminal is hard to use, but on terminal,
  terminal should probably be default."* A 94-column grid on a 411 dp screen is unreadable and the
  transcript is the answer; the same grid in the terminal a person typed `kampr` into is the thing
  they asked for, and answering it with a transcript is a worse herdr. So the **screen size** picks
  the default — phone-sized Compose clients open a talking pane on its conversation, desktop Compose
  and the CLI open the terminal — and the operator's own per-pane choice beats both. An agent pane
  with no adapter gets terminal view only, stated plainly, and a shell pane has no conversation at
  all.
- **The view is offered on the adapter, not on the transcript.** `has_conversation` is "a file
  resolves"; `converses` is "this node could read this harness", and it is true from the moment the
  session opens. Gating the view on the first hid the conversation for the whole gap between a
  session starting and its first prompt — the window in which somebody most wants to talk to it —
  so a pane that `converses` opens a conversation that says it is empty until turns arrive.
- **Adapters are keyed on Herdr's own `agent` string** (#38), and registered only if their root
  directory exists — so `caps.conversation` and a pane's `has_conversation` are answered from the
  same registry and a pane can never claim a conversation the node cannot serve.
- **Markdown is passed through verbatim; the client renders it.** A table stays a table because
  nothing in the pipeline ever had a chance to flatten it.
- **`pending` is sourced from the screen and says so.** The wire carries `source: "screen"` or
  `"transcript"` and **clients must not care which** — the shape is identical either way. Claude's
  question comes from `pane.read visible`; Codex could come from the transcript, but one
  implementation is better than two, so the screen path is the one that exists and `source` is the
  only thing that differs.
- **The node decides whether a submit key follows an answer, per harness.** A client sends only the
  key it was offered. Probe #72 confirmed live that Claude takes effect on the bare digit for both
  its trust prompt and a real `Bash` permission dialog — including the one whose footer reads "Enter
  to confirm".
- **A tool turn is revised in place when its result lands.** Match by turn id and replace, never
  append, or every tool renders twice — which is what lets a long-running tool show as running and
  then done.
- **Turn order is the node's order.** Sorting by timestamp shuffles a resumed session; the repo's own
  fixture has a final record stamped three weeks before the ones above it, which is exactly the case
  that looks fine until it isn't.
- **Transcript roots are containment roots** — canonicalised, symlink-escape-proof, and closed to
  path syntax in a pane-supplied id. A pane id is request input and a transcript path is not derived
  from it without that check.

## Consequences

- **The conversation view is always slightly behind the live screen**, because it reflects flushed
  transcript records. Both views are shown; neither is hidden from the user.
- **A blocked prompt can be answered without a terminal stream at all.** The question is on screen
  and the answer is a one-shot `send_keys`; reading and writing are independent surfaces. That is
  what makes triage cheap on a phone.
- **Codex publishes no plaintext thinking.** Its `reasoning` records are always `encrypted_content`
  with an empty summary (#45), so there is nothing to render — and the Claude adapter drops thinking
  to match rather than showing one harness's inner monologue and not the other's.
- **`event_msg` records must not be parsed.** Only `response_item` carries the conversation; Codex
  duplicates it into `event_msg` one-for-one for its own TUI, so parsing both double-renders every
  turn (#45).
- **A `diff` block is not one dialect.** Claude sends a unified diff rebuilt from `structuredPatch`;
  Codex sends its `*** Begin Patch` envelope verbatim. One classifier covers both because they share
  line prefixes, but a renderer must not expect unified-diff headers.
- **A stale `agent_session` can point at the wrong harness.** Herdr keeps reporting the last session
  announced for a pane, so a relaunched pane can advertise one that has gone. The adapter returns
  "no conversation" rather than an error for a shell pane, an unknown harness, or a session whose
  agent disagrees with the pane's own.
- **Every new harness is a new adapter.** This is per-harness work forever, and it is bounded work:
  the wire shape does not change, so an adapter is a parser and nothing else.
- **Two histories exist for one session, with different fidelities**, and a user has to understand
  that the terminal view's scrollback and the conversation are not the same thing. They are labelled
  as separate views rather than blended, which is the honest presentation.

## What would justify revisiting

- **A harness that publishes structure over an API rather than a file.** That is strictly better than
  tailing a private JSONL format, and the adapter seam is where it would land.
- **Herdr exposing the harness's own structured output.** Herdr already installs hooks into 17
  harnesses; if it ever surfaced turns rather than screens, this decision collapses into consuming
  that and the adapters go away.
- **Nothing about a better parser.** "Recover the markdown from the grid" is on the roadmap's cut
  list and stays there. The information is not present to be recovered; a parser that appears to
  work is a parser whose failures are invisible.
