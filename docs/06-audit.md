# Completeness audit — re-verified 2026-08-21

The original audit was written on 2026-08-20 and then went unmaintained: half its items were fixed
and annotated **FIXED** on the strength of a test, and nothing re-read the wire afterwards. This
pass re-decided **every** numbered item and every bullet against the running system — a real
`kampr serve` on 127.0.0.1:8797, real herdr 0.8.2 sessions, real bytes captured off `/ws`, and where
the answer lived in the client, those exact bytes replayed through the compiled
`dev.kampr.shared.wire.Wire` and `dev.kampr.shared.model.*` classes.

Every verdict below carries the command that decided it, and says whether it was decided **by
execution** or **by reading**. Nothing is marked fixed on the strength of a test alone.

## The root cause, restated — and one place it is now proved wrong

> The tests were written from the same mental model as the code, so both sides of a seam agree with
> each other and neither agrees with the wire.

Still the right diagnosis. The clearest new instance is item **N1** below: `workspace.create` works
in `LiveNodeTest` and fails against a real herdr, because the test always passes an `env` map and
the client's own New sheet never does.

The one thing that has genuinely improved: **`herd.patch` now survives a real round trip.** Node
bytes → `Wire.decode` → `KamprStore.accept` carries `agent_status` transitions and `removed_ids` into
the herd model. That was item #5, and it is the first seam in this repo proved end to end rather than
asserted on both sides.

Three structural facts, updated:

- **CI now proves something about the node.** `.github/workflows/ci.yml` installs herdr from
  `herdr.dev/latest.json`, checks the hash, and fails the job if the suite prints
  `skipping: no herdr on PATH`. That half is closed.
- **CI still proves nothing about the client against a real node.** `client/shared/src/jvmTest/.../LiveNodeTest.kt`
  is exactly the missing test — and it returns on its first statement unless `KAMPR_URL` and `KAMPR_TOKEN` are set,
  and CI runs `./gradlew build` with neither. The highest-value test in the repo exists and never runs.
- **The roadmap tick state is still worthless.** Unchanged.

## Broken

Of the nineteen: **thirteen are genuinely fixed and were re-broken unsuccessfully**, **three are
still open**, **one (#15) was wrong as written and hides a worse defect**, and **two (#4, #19) are
fixed in the main and carry a named residual**.

| # | Defect | Verdict | What decided it |
|---|---|---|---|
| 1 | Revoking a device does not disconnect it | **FIXED — execution** | Paired a device, opened `/ws`, watched a pane, then `POST /api/devices/<id>/revoke` from another device at t=+5.93 s. The already-open socket received `error{revoked}` and `CLOSE` in the same 10 ms tick. Not "until the socket happens to drop" |
| 2 | Binds `0.0.0.0` in cleartext by default | **FIXED — reading** | `crates/kampr-node/src/config.rs:161` — `Config::bootstrap` writes `127.0.0.1:{DEFAULT_PORT}`; `kampr init` prints the "this machine only" line and names `--bind` as the opt-in |
| 3 | A read-only device can promote itself to full | **FIXED — execution** | With a read-only token: `POST /api/pair {"role":"full"}` → **403**, `POST /api/devices/<id>/role` → **403**, `POST /api/devices/<id>/revoke` → **403**, `GET /api/devices` → **403**. On the socket, `input` / `manage` / `answer` / `notify` all returned `error{not_writer}` and the `echo PWNED` never reached the PTY. `kampr pair` with a device already enrolled prints an **unarmed** code and says so |
| 4 | The conversation server side does not exist | **FIXED, with a residual — execution** | `watch{conversation:true}` on the user's live `claude` pane returned `convo` with 2 turns, and `convo.load` paged it. **Residual:** `state.rs:325` derives `has_conversation` from the *agent kind*, not from a transcript existing. A `claude` started 10 minutes ago with no `~/.claude/projects/<slug>/*.jsonl` yet still advertises `has_conversation:true`, and `convo.load` answers `error{not_found, "no conversation open for this pane"}` — the blank Conversation pane the original item described, now only for a freshly started agent. That is the pane the client's own New sheet creates |
| 5 | **`herd.patch` is structurally incompatible** | **FIXED — execution, end to end** | Captured real `herd.patch` frames off `/ws` (`{"t":"herd.patch","changed":{"panes":[{…,"agent_status":"working"}]}}`, `added.nodes`, `removed_ids`), then replayed the **file of captured bytes** through the compiled `Wire.INSTANCE.decode` and `KamprStore.accept`. Result: `HerdPatch -> w1:p1=working`, then `=idle`, then `panes 10 → 9` on a removal. Node and client agree on the wire |
| 6 | **Three of four auth HTTP paths do not exist** | **STILL OPEN — execution** | Against the real router: `GET /auth/status` → **200 `text/html`** (the SPA index, via `.fallback(get(static_asset))`), `GET /auth/devices` → **200 `text/html`**, `POST /auth/devices/<id>/revoke` → **405**, `POST /auth/pair` → 200. See below for what has changed about the *symptom* |
| 7 | `kampr.db-wal` world-readable, unsalted SHA-256 | **FIXED — execution** | On disk: config and state dirs `drwx------`, `kampr.db`, `-wal`, `-shm`, `audit.jsonl`, `vapid.pem`, `node.key` all `-rw-------`. And the digests are not SHA-256: `sha256(normalise_code(code))` for four known codes is absent from the `pairings` table; `secret.rs` stretches with Argon2id. **Residual, by design:** the salt is a constant (`kampr/pairing/v1`), because the lookup is by digest |
| 8 | `trust_proxy` reads the leftmost `X-Forwarded-For` | **FIXED — execution** | With `trust_proxy = true`, three requests with heads `9.9.9.9` / `1.1.1.1` / `2.2.2.2` and the same appended hop `8.8.8.8` were all audited as `"peer":"8.8.8.8"`. A rotating header no longer buys a fresh bucket |
| 9 | The pairing `attempts` counter is read but never written | **FIXED — execution** | Five wrong redemptions → `attempts=5` on every outstanding row in `pairings`, and the *correct* code was then refused `429 too many attempts` |
| 10 | **The node errors on unknown `t`** | **STILL OPEN — execution** | Sent `{"t":"future.thing","whatever":1}` on a live socket. Got back `error{"bad_request","unknown variant \`future.thing\`, expected one of \`watch\`, \`unwatch\`, \`input\`, \`answer\`, \`convo.load\`, \`resync\`, \`ping\`"}`. `session.rs:240` says *"Unknown `t` values are ignored rather than refused"* and `session.rs:247` then hands the value to `serde_json::from_value::<ClientMsg>`, whose tagged enum refuses it. Two lines apart |
| 11 | `answer` never sends Codex's submit key | **FIXED — reading only** | `session.rs:1010` `SUBMIT_KEYS = [("codex","\r")]`, `session.rs:510` extends the keystrokes, and the loop at 511–521 issues them as two separate `registry.write` calls so herdr cannot coalesce them into a paste. **Not re-driven against a live codex this round** — the earlier probe (#79) stands unreconfirmed |
| 12 | **`applyReset` appends to the link table** | **STILL OPEN — execution, end to end** | Drove a real pane to emit OSC 8 links, unwatched and re-watched to force resets, captured the frames, then replayed them through the compiled `PaneState`. The node's semantics are asymmetric on purpose (`registry.rs:422` sends the *whole* table on reset; `registry.rs:489` sends only the *suffix* on a patch) and `PaneState.applyReset`/`applyPatch` both do `links += msg.links`. Wire in: `reset[]`, `patch[AAA]`, `reset[AAA]`, `reset[AAA]`, `patch[BBB]`, `reset[AAA,BBB]`. Client table out: `[AAA,AAA,AAA,BBB,AAA,BBB]`. **Node link id 1 is BBB; the client resolves it to AAA.** `applyPatch`'s append is correct; `applyReset` must replace |
| 13 | The same-origin gate is self-satisfying under DNS rebinding | **FIXED — execution** | `/ws` upgrade matrix: no `Origin` + bearer → **101**; `Origin: http://evil.com` → **403**; correct `Origin` → **101**; `Host: kampr.evil.com` **and** `Origin: http://kampr.evil.com` (both attacker-controlled, the rebinding case) → **403**; no `Origin` + session cookie → **403**. Cross-origin `POST /api/pair` → 403 |
| 14 | `readonly` is not gated on `prefs` or `caps` | **FIXED — execution** | A read-only device's `prefs` write is accepted (and bounded by `MAX_PREFS_BYTES = 2048` for every role, `session.rs:33`); every write verb is refused. Bounded rather than gated, as the fix claimed |
| 15 | `manage.rs` drops the `Child` — a zombie per session | **WRONG AS WRITTEN — see N2** | Created three sessions via `manage{session.create}`, then `ps -eo stat --ppid <node>`: three `Sl` children, **zero `Z`**. Stopped one and re-checked: reaped cleanly. `tokio::process` puts a dropped `Child` on its orphan queue and reaps it on `SIGCHLD`. There is no zombie. There is something worse, and it is N2 |
| 16 | `kampr serve` needs herdr before it binds | **FIXED — execution** | A node with `[herdr] socket = "/nonexistent/herdr.sock"` and `sessions = []`: `GET /healthz` → 200, `GET /` → 200, and the `herd` frame reads `online:false, detail:"/nonexistent/herdr.sock: connecting to herdr socket …"`. It binds, it serves, and it says why the herd is empty |
| 17 | A herd outage was invisible | **FIXED — execution** | Watched a pane in a named session, then `server.stop` on that session's socket at t=+8.000. At t=+8.13 the client had `error{herdr_unavailable}`, `error{node_offline}` and a `herd.patch`, then further patches at +11.1 and +17.1 |
| 18 | Only one herdr session was ever served | **FIXED — execution** | One node served five session nodes at once (`auditnode`, `/kampr-a1`…`/kampr-a5`), each its own `id`, `name` and `online`, discovered and dropped on the 15 s poll |
| 19 | Panes observed at the layout rect | **FIXED, with a residual — execution** | Split a pane in a headless session so the rect said 47 while `stty size` inside the PTY said `39 94`. The **grid is correct**: `grid.reset cols=94`, and a 94-character ruler arrived at full length, uncropped. **Residual:** `herd.panes[].cols` is still the rect for an *unwatched* pane — 47 against a real 94 — because `herdr_provider.rs:378` calls `known_cols`, which falls back to the rect when no measurement is cached. `rows` is the rect unconditionally (39 against a real 40). The client prints that number to the user in three places (`PaneScreen.kt:152`, `MosaicCell.kt:90`, `PanePicker.kt:109`). Rendering is unaffected — it uses `pane.cells.cols` from `grid.reset` |

### What has changed about #6's symptom

The original said the revoke "reports success while revoking nothing". Half of that is now false and
half is worse than it sounds:

- `AuthApi.revoke` **does** check `status.isSuccess()`, so it correctly returns `false` on the 405.
- `KamprApp.kt:113` then **throws the boolean away** and bumps `deviceRefresh`. Nothing reaches the
  user. The device stays connected and the UI says nothing at all — which is the same outcome the
  original described, arrived at by a different route.
- `DevicesScreen` renders **"No devices reported by this node."** on every run, for every node,
  because `/auth/devices` returns HTML that `DeviceList.serializer()` throws on and
  `runCatching{}.getOrNull()` swallows.
- `SetupScreen.kt:215` falls back to `"tier N"` and never shows the paired-device count.

And repointing the paths is **not** the whole fix. Two more layers of the same seam:

- `/api/devices` returns `created_at` and `last_seen_at`; `DeviceRecord` declares
  `@SerialName("added_at")` and `@SerialName("last_seen")`. Both would silently decode to `null`.
- **No endpoint serves `SetupStatus`'s shape at all.** It requires `address` (non-optional, no
  default); `/api/node` returns `build`, `bundle`, `enrolled`, `node_id`, `node_name`, `protocol`,
  `security` and nothing resembling `address`, `pairing_code` or `devices`. Pointing `status()` at
  `/api/node` swaps a 404-shaped failure for a decode-shaped one.

## Newly found this pass

| # | Defect | Verdict | What decided it |
|---|---|---|---|
| N1 | **`workspace.create` fails unless the client sends an `env` map.** `manage.rs:110` builds `json!({… "env": op.env …})` and `Option<Value>::None` serialises as `null`. herdr 0.8.2 rejects it: `invalid_request: invalid request: invalid type: null, expected a map`. The client omits `env` whenever it is empty (`Manage.kt`, `if (env.isNotEmpty())`), and `NewSheet.kt:103`/`:115` build exactly that — so **the New-workspace button fails every time unless the user typed an environment variable by hand** | **OPEN — execution** | Two `manage{workspace.create}` frames, one with `env:{"KAMPR_AUDIT":"1"}` → `managed{ok:true}` + a `herd.patch` carrying the new pane; one identical but without `env` → `managed{ok:false, code:"herdr_unavailable"}`. Confirmed directly on herdr's socket: omitting the `env` **key** succeeds, sending `"env":null` fails. `LiveNodeTest` passes `mapOf("KAMPR_LIVE" to "1")` and therefore never sees it. The whole family was swept — `tab.create`, `pane.split`, `pane.zoom`, `rename` (including the null-clear), `close`, `focus`, `agent.start`, `worktree.create`, `worktree.open`, `layout.export`, `layout.apply`, `session.create`, `session.stop` all tolerate their nulls. `env` is the only one |
| N2 | **A created herdr session is a foreground child in the node's process group and cgroup, and the systemd unit sets no `KillMode`.** `herdr server --session X` does not daemonise; it stays a live child of `kampr serve`. `SYSTEMD_UNIT` (`crates/kampr-cli/src/service.rs:401`) has `Type=simple`, `Restart=on-failure` and **no `KillMode`**, so systemd's default `control-group` applies: `systemctl --user restart kampr` SIGTERMs and then SIGKILLs **every herdr session the node ever created**, with the user's agents inside them. `PrivateTmp=yes` compounds it — those sessions inherit the node's private `/tmp`, so an agent in a node-created pane sees a different `/tmp` from the user's shell | **OPEN — execution** | `/proc/<node>/cgroup` and `/proc/<herdr child>/cgroup` are byte-identical, and both report the same `pgid`. Killing the node directly (not via systemd) re-parented the sessions to pid 1 and they survived — so the exposure is specific to the cgroup kill, which is exactly how the shipped unit runs |
| N3 | **The client decodes `scrollback.capped` and `scrollback.complete` and no UI reads either.** The node's honest gap reporting — the thing the original audit called out as careful — reaches the client and dies there | **OPEN — reading** | `Codec.kt:73` and `Protocol.kt:170` decode them; the only other occurrences of `capped` in the whole client tree are a test fixture and a bench workload. No screen, no strip, no badge |
| N4 | **After a burst outruns the poll, the node's history is permanently ~1000 rows while herdr still holds everything.** `pane.read recent` caps at 1000 and takes no offset (probe #51), so the ring can only be grown by stitching; once a gap discards, nothing backfills | **OPEN — execution** | `seq 1 50000` in a watched pane. herdr afterwards reports `max_offset_from_bottom: 50057`. The node's `scrollback` frame reports `total_rows: 961, complete: true, capped: true`. The client is told, correctly, that it has everything the node has — and, correctly, that more is unreachable. See the gap-sentinel decision below |
| N5 | `session.create` and `session.stop` return a `managed.id` shaped like a pane id — `<node_id>/kampr-a1` — for something that is a session name, because `manage::created_id` is passed through `session.global_pane` | **OPEN — execution** | `{"id":"01M0GKXXY86Y39D73BVGB1Q33Z/kampr-a1","ok":true,"op":"session.create"}`. Cosmetic today; a client that treats `Managed.id` as a pane id (as `LiveNodeTest` does for `workspace.create`) will address a pane that does not exist |

## Incomplete

- **Per-device prefs never restore.** **STILL OPEN — execution.** A fresh connection receives
  `hello` then `herd` and nothing else; no `prefs` is pushed. And the one-key write is destructive:
  `prefs{pane, {"view":"conversation"}}` then `prefs{pane, {"zoom":"2"}}` leaves the stored blob as
  `{"zoom":"2"}` — the view is gone. Both halves of the original bullet, proved.
- **PWA.** **The never-built table below was wrong and is corrected.** `manifest.webmanifest` and a
  224-line `sw.js` both ship. The service worker deliberately does **not** proxy the app's own
  fetches — it says so in its own header, and the reasoning (a second, worse cache in front of an
  immutable wasm bundle) is right. So there is a manifest and a worker, **no install prompt**
  (`beforeinstallprompt` appears nowhere in the tree) and **no offline shell, by choice**.
  `security.installable` is now closer to honest than the original bullet allowed.
- **The install path is built and has never run.** **STILL OPEN — execution.** `git tag` is empty.
  `release.yml` is tag-triggered (`on: push: tags: ['v*']`), and it pins cosign's certificate
  identity to `release.yml@refs/tags/…` so a `workflow_dispatch` run builds but never signs or
  publishes. With no tag, the signing and publication half has never run — by construction.
- **`min_herdr_version` is enforced by `kampr doctor` and nowhere else.** **STILL OPEN — reading.**
  `MIN_HERDR_VERSION` appears only in `crates/kampr-cli/src/doctor/herd.rs`. `kampr-node` never
  compares a version to anything; `state.rs:317` only *reports* `herdr_version` into the herd.
- **No protocol-version negotiation.** **STILL OPEN — reading.** `hello.protocol` is decoded into
  `ServerMsg.Hello.protocol` and there is not one read of it anywhere in the client.
- **Accessibility gaps that remain.** **Carried forward unverified.** The measurements (23 dp column
  indicator, 36 dp segmented control, 42 × 44 dp key caps) were not re-taken this round; no emulator
  was run. Treat them as of 2026-08-20 until someone re-measures.
- **Warm resume is per-process only.** **STILL OPEN — reading.** Nothing in the client persists a
  grid, a herd or a transcript across a process restart.
- **Silent failure on a flaky link.** **STILL OPEN — reading.** `KamprConnection.kt:51`
  `Channel(Channel.BUFFERED)` — 64 slots — and `:65` `outbox.trySend(msg)` with the result
  discarded. A dropped keystroke is unsignalled. And `AppState.kt:111` is still
  `AuthApi(client, target).pair(code) ?: code`: a failed pair persists the **pairing code** as the
  device token, producing the silent auth-failure loop.
- **Observability.** **STILL OPEN — execution.** `/healthz` is `get(|| async { "ok" })` and returns
  the literal string. No metrics endpoint. `HistoryStatus` is commented "Diagnostics only; nothing
  on the wire" and has no reader.
- ~~**Audit log holes.**~~ — **CLOSED — execution.** Every hole named is filled. Failed auth:
  `auth.rejected` on a bad bearer token, `pairing.rejected` on a wrong code,
  `pairing.rate_limited` when the limiter bites. `watch` is recorded with
  `{"scrollback":…,"conversation":…}`. `manage` carries `cwd`, `env`, `label`, `node`, `kind`,
  `name` — e.g. `{"cwd":"/tmp","env":{"KAMPR_AUDIT":"1"},"label":"withenv","node":"…","op":"workspace.create"}`.
  Rotation exists (`audit.rs:129`, `rotate()` at `:138`, with a test).
- ~~Missing outright: destructive-command confirm, threat model, ARCHITECTURE.md, ADRs.~~ —
  **ALL FOUR NOW EXIST — reading.** `client/terminal/.../guard/Destructive.kt` (321 lines),
  `SubmitGuard.kt`, `ConfirmSheet.kt` and `DestructiveTest.kt`; `docs/08-threat-model.md`;
  `ARCHITECTURE.md` (in flight — uncommitted at the time of this pass); ten ADRs under `docs/adr/`.
- **Mesh.** The bullet's closing caveat is stale: **the client half of the latency indicator (P4.9)
  is drawn** — `HerdPieces.kt:100/107`, `MosaicCell.kt:76`, `MosaicSwitcher.kt:136`,
  `PanePicker.kt:70`. What one machine still cannot prove is unchanged: real latency, a real NAT, a
  real reverse proxy.
- **Notifications.** Unchanged and still true: no physical device has run any of it, and the iOS
  Add-to-Home-Screen detection is written against documentation.

## The three items the file flagged as decisions

### A mid-session demotion is still not announced — **confirmed open, by execution**

Connected as `role: full`, watched a pane, then `POST /api/devices/<id>/role {"role":"readonly"}` at
t=+6.0. The socket received **nothing**: no `hello`, no `role`, no `error`. At t=+12.0 the client
sent an `input` and got `error{not_writer}`. Its `hello.role` said `full` for the whole session.

The decision is unchanged and now has a timeline attached to it. It has to be made before any UI
trusts `hello.role`, because that UI will be correct for exactly as long as nobody demotes anyone.

### The VT-emulator payoff is still not collected — **confirmed, by reading**

`kampr-term` exposes `new`, `feed`, `reset`, `resize`, `grid`, `cursor`, `take_dirty`. That is all of
it. Selection is `client/terminal/.../render/Selection.kt`, link resolution is `PaneState.links` plus
`CellBuffer.linkAt`, soft-wrap joining is in the client, and **there is no find anywhere in the tree**
— on either side. Either collect the benefit or stop claiming it. Unchanged.

### The gap sentinel — the phenomenon is real; **the arithmetic in this file was wrong**

The old bullet read: *"a 3 s poll against a 1,000-line cap means any output over ~330 rows/second
discards history — that is `cargo build`, not an edge case."* Both halves are wrong, and the
conclusion survives anyway.

**Wrong, part one: there are two polls and only one of them matters.** `SCROLLBACK_POLL = 3 s`
(`session.rs:20`) governs how often the *node* pushes ring deltas to a client — the ring holds 20 000
rows, so 3 s costs latency, not history. The poll that races herdr's 1000-row cap is the registry's,
and it is **adaptive**: `HistoryPolicy { row_budget: 400, fastest: 100 ms, quiet: 2 s }`, cadence
derived from a smoothed measured row rate. The floor puts the theoretical ceiling at 10 000 rows/s,
not 330.

**Wrong, part two: measured, the real threshold is between 1 200 and 2 500 rows/s.** A pane fed a
paced, numbered stream, with every delivered `scrollback` row reassembled and checked for holes:

| rate | rows emitted | rows delivered | internal gaps |
|---|---|---|---|
| 300 /s | 3 000 | 2 962 | **0** |
| 1 200 /s | 7 200 | 7 162 | **0** |
| 2 500 /s | 12 500 | — | **1** |
| 5 000 /s | 20 000 | 7 545 | **3** |

(The 38 missing rows at 300 and 1 200 are the ones still on screen rather than in history.)

**Wrong, part three: `cargo build` is nowhere near it.** Timed on this workspace with one crate
touched: **5 lines in 1.9 s — 2.6 rows/s**, three orders of magnitude under the threshold. A full
clean build with warnings is tens of rows/s, not thousands.

**Right anyway.** The failure is real and easy to reach — `seq 1 50000` does it, so does any `find /`,
any verbose test suite, any `cat` of a large file. And the failure is *permanent*: after that burst
herdr still reported `max_offset_from_bottom: 50057` while the node's frame said
`total_rows: 961, complete: true, capped: true`, and `pane.read` has no offset, so nothing will ever
backfill it. The client is told the truth on the wire and **shows none of it** (N3).

So the decision stands, with a better justification and a cheaper first move: **wire the `capped`
flag into the UI before adding a sentinel.** The node is already honest; the client is already
listening; nobody is reading. That is a one-screen change, and it is the difference between "the
history is short" and "the history is short and Kampr knows it".

## Planned and never built — corrected

Checked against code on 2026-08-21.

| Item | Status |
|---|---|
| Destructive-command confirm (P2.10) | **Built** — `guard/Destructive.kt`, `SubmitGuard.kt`, `ConfirmSheet.kt`, `DestructiveTest.kt`. The old "no occurrence of 'destructive' anywhere in the tree" is stale |
| Recovery code (P3.5) | **Done** — `kampr recover`, ~99 bits, single use, argon2id digest under its own salt domain |
| `kampr doctor` (P7.7) | **Done** — 11 checks, `--json`, non-zero exit on a real failure |
| Light theme / `prefers-color-scheme` (P6.8b) | **Built** |
| Per-theme ANSI palette (P6.8c) | **Built** ([ADR 0009](./adr/0009-the-terminal-keeps-its-own-ground.md)) |
| Accessibility (P6.11) | **Built**, with the gaps listed above unverified this round ([ADR 0010](./adr/0010-the-grid-is-described-not-read-out.md)) |
| PWA manifest + service worker (P6.10) | **Built** — `manifest.webmanifest` and `sw.js` both ship. No install prompt; no offline shell, deliberately |
| Kampr split view / mosaic (P4.5.8/P4.5.9) | **Built** — `client/mosaic` with `MosaicCell`, `MosaicSwitcher`, `PanePicker`, `MosaicLiveTest` |
| Mesh latency indicator (P4.9) | **Built** — drawn in four places |
| Kampr on Android as a *provider* (Phase 8.5) | Not started |
| ARCHITECTURE.md, ADRs, threat model (P3.1, P9.2, P9.3) | **Written** — `ARCHITECTURE.md`, ten ADRs, `docs/08-threat-model.md` |
| Terminal find | **Absent on both sides**, and named in the VT-emulator argument as one of its three payoffs |
| Release tag | **Never pushed** — `git tag` is empty |

## Verified good — re-checked

Still true, and three of these were re-proved rather than re-read:

- `kampr-term` matches Herdr's grid exactly.
- The `Outbox` backpressure design, including the `scrollback`/`styles` purge exemption.
- The scrollback stitching, and its gap reporting — **honest on the wire, and unread by the client
  (N3)**.
- `ModeSelector` implements probe #62 with hysteresis.
- WebAuthn ceremony hygiene.
- JSONL audit injection is impossible, the 0600 is real (**re-verified on disk**), and the log now
  covers failed auth, watch and full manage detail, and rotates.
- **New to this list:** the same-origin gate (#13), the revocation path (#1) and the herd-outage
  path (#17) all behave under a real attacker-shaped probe, not just under their tests.
- The probe log is still the best artefact in the repo.

## What would have caught what

The one test the original audit asked for — start the real node, connect the real client codec,
assert on bytes — **exists** as `LiveNodeTest`, and would have caught #5 and #12 today. It did not,
because CI runs it with no `KAMPR_URL` and it returns before doing anything. It would still not have
caught **N1**, because it passes an `env` map the real client never sends.

Two things belong in CI, in this order:

1. Run `LiveNodeTest` against a node CI already knows how to install herdr for. The node half of
   that job is solved; the client half is one environment variable and a `kampr init`.
2. Make it drive the ops **the way the client's own screens drive them** — `NewSheet` with no env
   row, `PaneActionsSheet` with no ratio — rather than the way the test author found convenient.
