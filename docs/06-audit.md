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

| # | Defect | Owner |
|---|---|---|
| 1 | **Revoking a device does not disconnect it.** The session captures `Device` at handshake and never re-reads it, so a revoked or demoted device keeps writing until the socket happens to drop. Both "covering" tests dodge it | `kampr-node`, `kampr-auth` |
| 2 | **Binds `0.0.0.0` in cleartext by default**, reversing P3.11 and crossing the Phase 3 gate | `kampr-node` |
| 3 | **A read-only device can promote itself to full.** `/auth/pair` is unauthenticated and the pairing code is printed into a Herdr pane, which read-only devices watch by design | `kampr-node`, `kampr-cli` |
| 4 | **The conversation server side does not exist.** `kampr-journal` is 1,823 lines with four items used; `caps.conversation` is hardcoded false; `convo.load` answers `unsupported` — yet agent panes *default* to Conversation, so they open blank | `kampr-node` |
| 5 | **`herd.patch` is structurally incompatible** — node emits arrays, client decodes objects, the throw is swallowed. No incremental herd update ever reaches the UI, including every `agent_status` change | both |
| 6 | **Three of four auth HTTP paths do not exist** (`/auth/*` vs `/api/*`). The revoke POST 405s and **reports success while revoking nothing** | `client/shared` |
| 7 | `kampr.db-wal` is world-readable and carries unsalted SHA-256 digests of ~39.6-bit pairing codes | `kampr-auth` |
| 8 | `trust_proxy` reads the **leftmost** `X-Forwarded-For` entry — attacker-controlled even behind a correct proxy, defeating the rate limiter it exists to protect | `kampr-node` |
| 9 | The pairing `attempts` counter is read but never written | `kampr-auth` |
| 10 | **The node errors on unknown `t`**, violating the protocol's own forward-compatibility rule. The test that "covers" it skips past the error | `kampr-node` |
| 11 | `answer` never sends Codex's submit key, so answering a Codex prompt does nothing (probe #43) | `kampr-node` |
| 12 | `applyReset` **appends** to the link table instead of replacing it, so after any second reset link ids resolve to the wrong URL | `client/shared` |
| 13 | The same-origin gate builds its allow-list from the request's own `Host`, so it is self-satisfying under DNS rebinding — and fails open when `Origin` is absent | `kampr-node` |
| 14 | `readonly` is not gated on `prefs` or `caps`: unbounded disk writes, and an unrated process-spawn amplifier | `kampr-node` |
| 15 | `manage.rs` spawns `herdr server --session` and drops the `Child` without `kill_on_drop` — a zombie per created session | `kampr-node` |

## Incomplete

- **Phase 4 (mesh) is entirely absent.** No peer transport, no node-to-node auth, no relay. `NodeIdentity` exists; its only consumer prints a fingerprint.
- **Phase 4.5 has a server and no client.** All 13 `manage` ops are implemented and `caps.manage` is true; `ClientMsg.Manage` is never constructed anywhere in `client/`. `Managed` and `NodeCaps` are decoded and discarded. `Manage.fields` is typed `Map<String,String?>`, which cannot express `ratio`, `env`, `args` or `layout`.
- **Per-device prefs never restore.** The node replies only to a client `prefs` message and pushes nothing at `hello`; the client only ever writes. And a one-key write replaces the whole blob, so setting the view erases the zoom.
- **No PWA** (P6.10) — yet `security.installable` is computed and surfaced as "install to home screen", a promise nothing can keep. Phase 8's push design has no foundation.
- **No notifications** (Phase 8). `caps.push` is hardcoded false; `pane.agent_status_changed` is not subscribed.
- **The install path cannot work.** Both scripts fetch a release that no workflow produces, and neither verifies a checksum on a binary that grants RCE.
- **No `min_herdr_version` enforcement** anywhere, despite the manifest declaring a 0.8.2 floor.
- **No protocol-version negotiation.** `hello.protocol` is parsed and never read. One migration, no story for a second.
- **No accessibility.** Zero `semantics`/`contentDescription` in the whole client; no reduced-motion.
- **Warm resume is per-process only** — nothing is cached across an app restart, so the second open shows an empty grid.
- **Silent failure on a flaky link**: keystrokes `trySend` into a 64-slot channel and are dropped unsignalled; a failed pair persists the *pairing code* as the token, producing a silent auth-failure loop.
- **Observability**: registry accessors exist with no endpoint, `/healthz` returns a literal string, no metrics. `KamprStore.blocked()` — the triage list the roadmap called the one Collie idea worth stealing — has no callers.
- **Audit log holes**: nothing on failed auth (token probing is invisible), nothing on `watch`/scrollback (a read-only device exfiltrating every terminal leaves one line), and `manage` omits `cwd`/`env`/`args` — the fields that say what actually ran. No rotation.
- Missing outright: `kampr doctor`, recovery code, destructive-command confirm, threat model, ARCHITECTURE.md, ADRs, release workflow. `kamprctl.sh uninstall` keeps tokens, contradicting P7.8.

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
