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
- **Phase 4.5 has a server and no client.** All 13 `manage` ops are implemented and `caps.manage` is true; `ClientMsg.Manage` is never constructed anywhere in `client/`. `Managed` and `NodeCaps` are decoded and discarded. `Manage.fields` is typed `Map<String,String?>`, which cannot express `ratio`, `env`, `args` or `layout`.
- **Per-device prefs never restore.** The node replies only to a client `prefs` message and pushes nothing at `hello`; the client only ever writes. And a one-key write replaces the whole blob, so setting the view erases the zoom.
- **No PWA** (P6.10) — yet `security.installable` is computed and surfaced as "install to home screen", a promise nothing can keep. Phase 8's push design has no foundation.
- **No notifications** (Phase 8). `caps.push` is hardcoded false; `pane.agent_status_changed` is not subscribed.
- **The install path is built but has never run for real.** `.github/workflows/release.yml` now produces the artefacts both scripts fetch, `install.sh` refuses a binary whose `SHA256SUMS` entry is missing or wrong, and a keyless cosign signature is checked when cosign is present. No tag has been pushed, so the workflow itself — cross-compilation, signing, publication — is unexercised.
- **No `min_herdr_version` enforcement** anywhere, despite the manifest declaring a 0.8.2 floor.
- **No protocol-version negotiation.** `hello.protocol` is parsed and never read. One migration, no story for a second.
- **No accessibility.** Zero `semantics`/`contentDescription` in the whole client; no reduced-motion.
- **Warm resume is per-process only** — nothing is cached across an app restart, so the second open shows an empty grid.
- **Silent failure on a flaky link**: keystrokes `trySend` into a 64-slot channel and are dropped unsignalled; a failed pair persists the *pairing code* as the token, producing a silent auth-failure loop.
- **Observability**: registry accessors exist with no endpoint, `/healthz` returns a literal string, no metrics. `KamprStore.blocked()` — the triage list the roadmap called the one Collie idea worth stealing — has no callers.
- **Audit log holes**: nothing on failed auth (token probing is invisible), nothing on `watch`/scrollback (a read-only device exfiltrating every terminal leaves one line), and `manage` omits `cwd`/`env`/`args` — the fields that say what actually ran. No rotation.
- Missing outright: `kampr doctor`, recovery code, destructive-command confirm, threat model, ARCHITECTURE.md, ADRs.

## One wire gap the security pass opened, deliberately

**A mid-session demotion is enforced but not announced.** The client still holds the `hello` that
said `role: full` and learns only from an `error{not_writer}`. Telling it properly needs either a
re-sent `hello` or a new `role` message — and the protocol says `hello` is the *first* message on a
connection, so inventing one unilaterally was the wrong move. Decide it before the client grows any
UI that trusts `hello.role`.

## Planned and never built — the honest list

Checked against code on 2026-08-20, not against roadmap ticks (which are unmaintained).

### Notifications — Phase 8, entirely absent
The whole point of a phone client is being *told* an agent is blocked. None of it exists.

- `caps.push` is hardcoded `false` (`session.rs:505`); `web-push` is not in `Cargo.toml`.
- **`pane.agent_status_changed` is not subscribed** — `herdr_provider.rs:18` documents why (herdr rejects the subscription without a `pane_id`, and one bad entry rejects the whole call, probe #54), so status reaches the herd model only via the 3 s poll. That is the event the entire triage story rests on.
- No VAPID, no subscription store, no service worker, no deep link from a notification, no batching, no snooze, no per-agent mute.
- **`notification.show` is unused** (probe #50) — a phone cannot raise a toast on the desktop.
- `KamprStore.blocked()` — the triage list the roadmap called "the one Collie product idea worth stealing wholesale" — **has no callers**.
- On Android the native client needs **APNs-free FCM or UnifiedPush**, not Web Push; that fork was identified in §3.11 and never decided.

### Also planned, also absent
| Item | Status |
|---|---|
| Destructive-command confirm (P2.10) | No occurrence of "destructive" anywhere in the tree |
| Recovery code (P3.5) | Not implemented |
| `kampr doctor` (P7.7) | Not implemented |
| Light theme / `prefers-color-scheme` (P6.8b) | All four themes are dark |
| Per-theme ANSI palette (P6.8c) | One hardcoded 16-slot table shared by every theme |
| Accessibility (P6.11) | Zero `semantics` / `contentDescription` in the client |
| PWA manifest + service worker (P6.10) | Absent — yet `security.installable` is advertised |
| `manage` client UI (Phase 4.5) | Server complete; `ClientMsg.Manage` is never constructed |
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
