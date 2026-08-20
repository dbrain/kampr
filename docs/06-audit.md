# Completeness audit — 2026-08-20

An independent audit read the whole repo against the roadmap, the briefs and the probe log. Its
headline findings were verified by hand before being accepted. This file is the standing record of
what is broken, incomplete or missing; **it is not a wish list**, and an item leaves it only when the
fix has a test proving it.

## The root cause, stated once

> The tests were written from the same mental model as the code, so both sides of a seam agree with
> each other and neither agrees with the wire.

`herd.patch` is constructed in memory on both sides and never round-tripped. The revocation test
opens a *second* socket rather than checking the first. The unknown-`t` test skips past the very
error it should have caught. The client's auth API was written against paths a brief *described*
rather than paths the router *serves*.

**Every one of these would have been caught by a single test that starts the real node, connects the
real client codec, and asserts on bytes.** That test is the highest-value thing missing from this
repo, and it belongs in CI.

Two structural facts that let it happen, both worth fixing on their own:

- **CI green currently proves almost nothing about the node.** The 13 `kampr-node` integration tests
  skip silently on any machine without `herdr` on `$PATH`, which includes `ubuntu-latest`.
- **The roadmap tick state is worthless.** ~15 items ticked out of ~180, never updated after the
  first commit, with duplicate ids (P1.3, P1.4b, P1.6c each appear twice, once ticked once not).

## Broken

Ten of the fifteen are fixed, each with a test that fails first. The rest are open.


| # | Defect | Owner |
|---|---|---|
| 1 | ~~**Revoking a device does not disconnect it.** The session captures `Device` at handshake and never re-reads it, so a revoked or demoted device keeps writing until the socket happens to drop. Both "covering" tests dodge it~~ — **FIXED** — device re-read on a short interval plus a broadcast, and before every write verb; tests drive the already-open socket | `kampr-node`, `kampr-auth` |
| 2 | ~~**Binds `0.0.0.0` in cleartext by default**, reversing P3.11 and crossing the Phase 3 gate~~ — **FIXED** — loopback default, `--bind` is the explicit opt-in | `kampr-node` |
| 3 | ~~**A read-only device can promote itself to full.** `/auth/pair` is unauthenticated and the pairing code is printed into a Herdr pane, which read-only devices watch by design~~ — **FIXED** — a printed code is inert until an operator arms it at the console; wrong guesses burn it | `kampr-node`, `kampr-cli` |
| 4 | ~~**The conversation server side does not exist.** `kampr-journal` is 1,823 lines with four items used; `caps.conversation` is hardcoded false; `convo.load` answers `unsupported` — yet agent panes *default* to Conversation, so they open blank~~ — **FIXED** — `convo` / `convo.turn` are wire types, a pump per watched pane follows the transcript and revises turns by id, `convo.load` pages the cursor, and `caps.conversation` and every pane's `has_conversation` are both answered from the one adapter registry. Herdr never announces an `agent_session` (probe #75), so the transcript is resolved from the pane's cwd | `kampr-node` |
| 5 | **`herd.patch` is structurally incompatible** — node emits arrays, client decodes objects, the throw is swallowed. No incremental herd update ever reaches the UI, including every `agent_status` change | both |
| 6 | **Three of four auth HTTP paths do not exist** (`/auth/*` vs `/api/*`). The revoke POST 405s and **reports success while revoking nothing** | `client/shared` |
| 7 | ~~`kampr.db-wal` is world-readable and carries unsalted SHA-256 digests of ~39.6-bit pairing codes~~ — **FIXED** — state and config dirs 0700, db and both sidecars 0600, argon2id digests | `kampr-auth` |
| 8 | ~~`trust_proxy` reads the **leftmost** `X-Forwarded-For` entry — attacker-controlled even behind a correct proxy, defeating the rate limiter it exists to protect~~ — **FIXED** — reads the hop the proxy appended | `kampr-node` |
| 9 | ~~The pairing `attempts` counter is read but never written~~ — **FIXED** — `attempts` written; a run of misses burns outstanding codes | `kampr-auth` |
| 10 | **The node errors on unknown `t`**, violating the protocol's own forward-compatibility rule. The test that "covers" it skips past the error | `kampr-node` |
| 11 | ~~`answer` never sends Codex's submit key, so answering a Codex prompt does nothing (probe #43)~~ — **FIXED** — a per-harness table, written as two separate writes because a burst is coalesced into a paste (probe #79). Claude gets nothing, reconfirmed live on 2.1.237 against both its dialogs (probe #78) | `kampr-node` |
| 12 | `applyReset` **appends** to the link table instead of replacing it, so after any second reset link ids resolve to the wrong URL | `client/shared` |
| 13 | ~~The same-origin gate builds its allow-list from the request's own `Host`, so it is self-satisfying under DNS rebinding — and fails open when `Origin` is absent~~ — **FIXED** — allowlist derives from the bind address; cookie-without-Origin fails closed | `kampr-node` |
| 14 | ~~`readonly` is not gated on `prefs` or `caps`: unbounded disk writes, and an unrated process-spawn amplifier~~ — **FIXED** — bounded for every role rather than gated by role | `kampr-node` |
| 15 | `manage.rs` spawns `herdr server --session` and drops the `Child` without `kill_on_drop` — a zombie per created session | `kampr-node` |
| 16 | ~~**`kampr serve` needs herdr before it binds.** `Node::start` retried for 30 s at `debug!` and then exited, so with herdr down the port refused connections in silence and a service manager saw a restart loop — and the node could not serve its own "herdr is not running" page~~ — **FIXED** — the listener binds unconditionally and each herdr connection is an independent supervised loop that retries forever, Collie's two-recovery-loops shape | `kampr-node`, `kampr-core` |
| 17 | ~~**A herd outage was invisible.** Stopping herdr under a live watcher emitted no `error`, no `node_offline`, and left `herd.nodes[].online` true for the whole outage (probe #70) — the pane just froze~~ — **FIXED** — the node diffs *nodes* as well as panes, flips `online`, emits `herdr_unavailable` and `node_offline`, carries an operator-readable `detail`, and re-sends the whole `herd` on recovery | `kampr-node` |
| 18 | ~~**Only one herdr session was ever served.** `caps.sessions` advertised every named session on the host and `state.rs` connected to exactly one, so a user with `default` and `agents` saw one of them~~ — **FIXED** — one `Provider` per socket, each its own node in the herd model, discovered and dropped on a poll; `[herdr] sessions` restricts the set, defaulting to all | `kampr-node` |
| 19 | ~~**Panes were observed at the layout rect, which is fiction in a headless session.** After a split the rect said 47 while the PTY stayed 93, so `observe --cols 47` cropped and the browser showed the left half of every row (probe #68)~~ — **FIXED** — the width is measured from `pane.read` (`recent` against `recent_unwrapped`, probes #84–#87), which also fixes #69's padded column | `kampr-core` |

## Incomplete

- ~~**Phase 4 (mesh) is entirely absent.**~~ — **BUILT** — `crates/kampr-mesh`: outbound-dialling peers, a mutual ed25519 handshake over `/mesh` with single-use join codes and a pinned hub key, and a relay that keeps one `watch` per pane per link behind a shadow grid. Proved by `crates/kampr-node/tests/mesh.rs` — two nodes against two herdr sessions on one machine. **What one machine cannot prove: real network latency, a real NAT, and a real reverse proxy.** The client half of the latency indicator (P4.9) is still to draw.
- ~~**Phase 4.5 has a server and no client.**~~ — **FIXED** — `ClientMsg.Manage` carries a sealed `ManageOp` with one type per op, so `ratio`, `env`, `args` and `layout` are real JSON types; the client asks for `caps` on `hello` and again when the herd changes; `Managed` carries `code`/`message`/`layout`; and the New sheet plus the pane actions reach every op. The mosaic (P4.5.8/P4.5.9) is still missing.
- **Per-device prefs never restore.** The node replies only to a client `prefs` message and pushes nothing at `hello`; the client only ever writes. And a one-key write replaces the whole blob, so setting the view erases the zoom.
- **PWA is now half built** (P6.10): a manifest, icons and a service worker ship, which is what Phase 8 needed. There is still no install prompt and no offline shell, so `security.installable` remains a bigger promise than the client keeps.
- ~~**No notifications** (Phase 8). `caps.push` is hardcoded false; `pane.agent_status_changed` is not subscribed.~~ — **BUILT** — per-pane status subscription with debounced resubscribe (beats the poll by a mean 2.33 s over five live runs, probe #78), VAPID at `kampr init`, subscriptions in the device database, batching, the question in the body, snooze and mute; proved end to end with a real Firefox against Mozilla's push service. `docs/08-notifications.md`
- **The install path is built but has never run for real.** `.github/workflows/release.yml` now produces the artefacts both scripts fetch, `install.sh` refuses a binary whose `SHA256SUMS` entry is missing or wrong, and a keyless cosign signature is checked when cosign is present. No tag has been pushed, so the workflow itself — cross-compilation, signing, publication — is unexercised.
- **`min_herdr_version` is enforced by `kampr doctor` and nowhere else.** The floor is read off the live
  socket (`ping` carries `version` and `protocol`) and compared to the manifest's 0.8.2, with a test
  tying the constant to the manifest. The *node* still starts against an older herdr without
  complaint.
- **No protocol-version negotiation.** `hello.protocol` is parsed and never read. One migration, no story for a second.
- ~~**No accessibility.** Zero `semantics`/`contentDescription` in the whole client; no reduced-motion.~~ — **BUILT** (P6.11). Every interactive control routes through `Modifier.action` / `Modifier.gestureAction` and carries a name for the action rather than the glyph; agent status is a shape as well as a colour (`MarkShape`: square, disc, bar, ring) and a word; the triage list, the pending strip, the destructive-command sheet, the stale badge and the offline strip are live regions; `prefers-reduced-motion` is read per platform and gates the cursor blink, the momentum fling and the transcript's animated scroll. `SemanticsLayerTest` fails the build on a bare `clickable` or an animation that does not consult `LocalReduceMotion`. The terminal grid is [ADR 0010](./adr/0010-the-grid-is-described-not-read-out.md) — described, cursor line spoken, **no review mode**. Verified with TalkBack against a live pane (probes #92–#94).
- **Accessibility gaps that remain.** No terminal review mode and no braille (ADR 0010). Terminal selection is sighted-only — the handles are dragged at pixel positions. A sheet is modal by clearing the semantics of the screen behind it, not by a focus trap: Tab can still leave a sheet on desktop, though Escape closes one. Touch targets measured on a 411 dp portrait emulator: the **column indicator is 23 dp tall** (below both floors) and the **Terminal/Conversation segmented control is 36 dp** (meets the landscape rule, under the portrait one). The key row's caps are 42 × 44 dp — 44 dp tall, and 42 wide because eight caps and their gaps do not fit 44 dp each across 411 dp.
- **Warm resume is per-process only** — nothing is cached across an app restart, so the second open shows an empty grid.
- **Silent failure on a flaky link**: keystrokes `trySend` into a 64-slot channel and are dropped unsignalled; a failed pair persists the *pairing code* as the token, producing a silent auth-failure loop.
- **Observability**: registry accessors exist with no endpoint, `/healthz` returns a literal string, no metrics. (`KamprStore.blocked()` now has callers — `triage()` feeds the "Needs you" list on every breakpoint.)
- **Audit log holes**: nothing on failed auth (token probing is invisible), nothing on `watch`/scrollback (a read-only device exfiltrating every terminal leaves one line), and `manage` omits `cwd`/`env`/`args` — the fields that say what actually ran. No rotation.
- Missing outright: destructive-command confirm, threat model, ARCHITECTURE.md, ADRs.

## One wire gap the security pass opened, deliberately

**A mid-session demotion is enforced but not announced.** The client still holds the `hello` that
said `role: full` and learns only from an `error{not_writer}`. Telling it properly needs either a
re-sent `hello` or a new `role` message — and the protocol says `hello` is the *first* message on a
connection, so inventing one unilaterally was the wrong move. Decide it before the client grows any
UI that trusts `hello.role`.

## Planned and never built — the honest list

Checked against code on 2026-08-20, not against roadmap ticks (which are unmaintained).

### Notifications — Phase 8, **built**
Every line below was on this list. What replaced it is in `docs/08-notifications.md`; what is still
open is at the bottom.

- ~~`caps.push` is hardcoded `false`~~ — it now reports reality: secure context **and** a VAPID key **and** `push.enabled`. `security.push` says what the origin allows; `caps.push` says what the node can do.
- ~~**`pane.agent_status_changed` is not subscribed**~~ — subscribed **per agent pane**, with the list rebuilt whenever the agent-pane set moves and a 500 ms floor between resubscribes. Events poke the poll rather than replacing it, so a missed one costs an interval and never correctness. **Measured: the event beats the 3 s poll by a mean 2.33 s** across five runs against a real `claude` at a real permission prompt (probe #78). A stale `pane_id` turns out to be as fatal as a missing one (probe #76), which is why the retry re-derives from a fresh snapshot.
- ~~No VAPID, no subscription store, no service worker, no deep link, no batching, no snooze, no per-agent mute~~ — all present. Subscriptions live in the **device** database, so revocation is a `WHERE` clause rather than a cleanup job.
- ~~**`notification.show` is unused**~~ — wired twice: "Tell the desk" on a pane, and a pairing confirmation on the console the code was printed on. Attributed by the node, rate limited, and honest about `no_foreground_client` on a headless herdr (probe #77).
- ~~`KamprStore.blocked()` has no callers~~ — `triage()` feeds the "Needs you" list on portrait, landscape and desktop.
- ~~The Android fork was never decided~~ — **UnifiedPush**, because a distributor endpoint is an RFC 8291 endpoint and the node's sender is therefore unchanged: no Google project, no per-app secret. The server half is done; the client half needs a distributor the user installs, and is documented rather than assumed.

Still open: no physical device has run any of it, and iOS Add-to-Home-Screen detection is written
against the documentation rather than against an iPhone.

### Also planned, also absent
| Item | Status |
|---|---|
| Destructive-command confirm (P2.10) | No occurrence of "destructive" anywhere in the tree |
| Recovery code (P3.5) | **Done** — `kampr recover`, generated at `kampr init`, ~99 bits, single use, reissued on redemption |
| `kampr doctor` (P7.7) | **Done** — 11 checks, `--json`, non-zero exit on a real failure |
| Light theme / `prefers-color-scheme` (P6.8b) | **Built** — every theme has both grounds; `KamprTheme` resolves System/Dark/Light |
| Per-theme ANSI palette (P6.8c) | **Built** — 16 slots per theme on a dark terminal ground ([ADR 0009](./adr/0009-the-terminal-keeps-its-own-ground.md)) |
| Accessibility (P6.11) | **Built** — controls named, status shaped as well as coloured, live regions, reduced motion; terminal grid per [ADR 0010](./adr/0010-the-grid-is-described-not-read-out.md), with no review mode |
| PWA manifest + service worker (P6.10) | Absent — yet `security.installable` is advertised |
| Kampr split view (P4.5.8/P4.5.9) | The client-side mosaic over several nodes' panes; `manage` itself now has a client |
| Kampr split view / mosaic (P4.5.8) | Designed, not built |
| Kampr on Android as a *provider* (Phase 8.5) | Not started — distinct from the Android *client*, which is in flight |
| ARCHITECTURE.md, ADRs, threat model (P3.1, P9.2, P9.3) | Not written |

## Decisions worth revisiting

- **The VT-emulator-in-Rust decision was right; its stated payoff was not collected.** The argument was that selection, find and OSC 8 become node features rather than three client reimplementations. In practice selection, links and soft-wrap-joining are all implemented in the client, and there is no find at all. Either collect the benefit or stop claiming it.
- **"Preserving history across a gap needs a wire change, deliberately not in v1" should be decided now.** The condition was "evidence that gaps still hurt after adaptive polling". The evidence is structural: a 3 s poll against a 1,000-line cap means any output over ~330 rows/second discards history. That is `cargo build`, not an edge case. A gap sentinel is a two-field addition.

## Verified good

Worth recording so it is not re-litigated: `kampr-term` matches Herdr's grid exactly; the `Outbox`
backpressure design is sound, including the `scrollback`/`styles` purge exemption; the scrollback
stitching and its honest gap reporting are careful; `ModeSelector` implements probe #62 properly with
hysteresis; WebAuthn ceremony hygiene is correct (single-use challenges, UV required, counter
regression handled); JSONL audit injection is genuinely impossible and the 0600 mode is real. The
probe log is the best artefact in the repo.
