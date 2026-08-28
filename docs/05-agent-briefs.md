# Agent briefs

Six workstreams that can run in parallel **once the contracts exist**. Read
[`04-wire-protocol.md`](./04-wire-protocol.md) first — every brief is downstream of it.

## Ground rules for every brief

1. **The probe log is the source of truth about Herdr.** [`03-probe-log.md`](./03-probe-log.md). If
   you need a fact about Herdr that isn't there, probe it and add a row — do not assume, and do not
   trust a memory of another terminal multiplexer.
2. **Looking at a pane never reshapes it**, and only the `pane.size` op reshapes one on purpose
   (probe #17, #18; [ADR 0012](adr/0012-one-deliberate-resize-behind-a-panel.md)). Rendering handles
   small screens: zoom, pan, and the conversation view.
3. **Zero comments by default.** Names and types are the documentation. Comment only a non-obvious
   *why* — a constraint, a workaround, an invariant a reader cannot guess. The existing crates show
   the intended density.
4. **Bug fixes and logic changes are TDD.** Failing test first, at integration level where possible.
5. **Stay inside your seam.** If you need a change in someone else's crate, write it down and raise
   it; do not reach across.
6. **`cargo test && cargo clippy --all-targets` clean before you call anything done.**

## What already exists

| Path | State |
|---|---|
| `crates/kampr-herdr` | **Working.** Socket RPC, `session.snapshot` model, native geometry from the layout rect, `observe` supervisor with NDJSON decode, `send_text`/`send_keys`, scrollback read with the safety interlock |
| `crates/kampr-term` | **Working.** `vte`-based emulator → cell grid, dirty-row tracking, OSC 8 interning, 9 tests |
| `crates/kampr-spike` | **Working.** Proves `observe` → emulator → grid reproduces Herdr's own grid exactly (probe #41). Keep it building; it is the pipeline's canary |

---

## A — Node core: providers, stream supervision, scrollback

**Owns** `crates/kampr-herdr`, `crates/kampr-term`, new `crates/kampr-core`.

- Extract a `Provider` trait — `list_panes`, `watch_pane`, `write_pane`, `read_scrollback` — with
  Herdr as the first implementation. This is what later allows an Android local-PTY provider; it is
  cheap now and expensive to retrofit.
- Pane registry: one emulator per pane, refcounted by watchers, torn down when the last one leaves.
- Restart the observer when native geometry changes. Subscribe `layout.updated`; re-derive from
  `layouts[].panes[].rect`; emit a fresh `grid.reset`.
- Scrollback: backfill via `read_scrollback` **only** when `Pane::scrollback_is_safe_to_read`, feed
  it through the same emulator so styling matches, expose it as absolute row indices.
- Reconnect Herdr with backoff; a dropped socket must never lose a client's session.
- Style-table interning and `RowDiff` → wire `Run` conversion live here.

**Done when** two clients can watch the same pane, one restarts, and both get correct grids; and a
shell pane with 400 lines of scrollback delivers all 400 with colour intact.

---

## B — HTTP/WS surface and auth

**Owns** `crates/kampr-node` (axum 0.8.9), `crates/kampr-auth`.

- axum server, WebSocket at `/ws`, protocol exactly as specified. Per-client style tables.
- Backpressure: bounded per-client queue; on overflow **drop patches and send one `grid.reset`**,
  never buffer without limit.
- Config: bind address (loopback default), `trust_proxy` opt-in for `X-Forwarded-*`, own-TLS mode
  (rustls 0.23.43) as an alternative to a reverse proxy.
- Tier 0 auth: pairing code → device-bound token, LAN bind, expiry.
- Tier 1+ auth: WebAuthn via `webauthn-rs` 0.5.5, configurable RP ID.
  **A WebAuthn RP ID must be a registrable domain — an IP address is not one, and HTTPS does not
  change that.** Detect the tier and hide what cannot work rather than failing at the last step.
- Devices table, roles (`full`/`readonly`), revocation, rate limiting, JSONL audit log at 0600.
- Same-origin gate; strict CSP.

**Done when** a phone on the LAN pairs with a code and drives a pane, and the same node behind a
reverse-proxy hostname enrols a passkey.

---

## C — Client shell (Compose Multiplatform)

**Owns** `client/shared` navigation + herd surfaces, `client/androidApp`, `client/iosApp`,
`client/webApp`. Kotlin 2.4.10, CMP 1.11.1, Ktor client 3.5.2.

- Design tokens first, mirroring `docs/design/build.py`: `soft` ships, `phosphor` / `warm` /
  `brutalist` defined alongside. No colour, font or radius literal outside the token layer.
- Herd navigator grouped by node with status and latency; three breakpoints — desktop, portrait,
  landscape — matching the artboards.
- WS client: `hello` → `herd` → watch/unwatch, reconnect with backoff, cached-render-then-swap on
  reconnect (never a spinner).
- Setup ladder screen; device list; theme switcher.

**Done when** the herd navigator matches the artboards at all three breakpoints against a live node.

---

## D — Terminal renderer (Compose Multiplatform)

**Owns** `client/shared/terminal`. The highest-risk client work — start with the spike.

- **Spike first, before anything else**: draw a 74×30 cell grid at 60 fps on wasm and on a
  mid-range Android. If wasm cannot hold it, say so immediately — it changes C's plan too.
- Apply `grid.reset` / `grid.patch` / `styles` into a cell buffer; draw with Compose Canvas.
- Pinch-zoom and pan over the native grid, column indicator, follow-cursor, presets
  (fit width / readable / close up), persisted per pane per device.
- Key row: two rows portrait, one row landscape, latching Ctrl/Alt/Shift/Fn, long-press alternates.
  Everything Herdr's key grammar rejects goes out as its escape sequence via `input.text`
  (probe #8/#9).
- Live input: offscreen contenteditable read through `beforeinput`/`input` and diffed —
  **Android soft keyboards do not give usable `keydown`**. Handle `composition*`; disable
  autocapitalize, autocorrect, autocomplete, spellcheck.
- Key row docks to the OSK via `visualViewport`; it never replaces the keyboard, so dictation keeps
  working.
- Scrollback rendering above the live grid where the node supplies it.

**Done when** you can drive `vim` and answer a Claude permission prompt from a phone, with
characters echoing from the pane rather than from a local buffer.

---

## E — Conversation: transcripts and markdown

**Owns** `crates/kampr-journal`, `client/shared/conversation`.

- Adapter registry keyed on Herdr's `agent` string, the way Collie's `bridge/journal/` does it.
- Claude adapter: `~/.claude/projects/<slug>/<uuid>.jsonl`. Schema confirmed (probe #39).
- Codex adapter: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`. **Schema unverified — parse it
  before designing around it.**
- Transcript roots are containment roots, never request input.
- Tail-follow with incremental parse; new turns push over the existing WS.
- **Resolve probe #40 early**: is a pending tool request in the transcript *before* approval? If not,
  the node sources `pending` from `pane.read visible` and sets `source:"screen"`. Either way the
  wire shape is unchanged.
- Client rendering: tables as real tables (horizontally scrollable), fenced code with highlighting
  and copy, collapsible tool calls, diffs as diffs, whole-transcript search.

**Done when** a Claude session with a markdown table renders as a table on a phone, and its pending
prompt can be answered without any terminal stream.

---

## F — Packaging, install, release

**Owns** `packaging/`, `herdr-plugin.toml`, CI.

- `herdr-plugin.toml`: `[[actions]]` start/stop/restart/status/url/update/uninstall, `[[panes]]`
  popup setup (`placement = "popup"`, `width = "80%"`), `[[startup]]` nudge, `min_herdr_version`.
- **`[[build]]` downloads a prebuilt binary rather than compiling** — no Rust toolchain on a user's
  machine. Plugin v1 has no `plugin update` and a reinstall restarts nothing, so `update` is ours.
- Standalone: `install.sh` → `kampr init` → `kampr service install` (systemd `--user`, launchd).
- Single binary: Gradle builds the CMP wasm bundle, `rust-embed` bakes it in.
- CI: `cargo test`, `cargo clippy --all-targets`, Gradle build, plugin manifest validation,
  release artefacts for linux/macos × x86_64/aarch64.

**Done when** a clean machine goes from one command to a working paired phone session.

---

## Sequencing

```
        ┌─ A (node core) ──┬─ B (surface + auth) ─┐
probe   │                  │                      ├─ F (packaging)
gaps ───┼─ E (transcripts)─┘                      │
        │                                          │
        └─ C (client shell) ── D (terminal render) ┘
```

D's wasm spike and E's probe #40 are the two answers that can still change a plan. Get both early.

**Verification is not "it compiles".** For each brief: grep for call sites of every new
function — dead code is a gap; trace one path end to end; and re-read the brief line by line against
what actually exists.
