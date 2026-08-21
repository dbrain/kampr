# Completeness audit — re-decided 2026-08-21 (second pass)

The first pass of this file was written on 2026-08-20 and went unmaintained until the morning of
2026-08-21, when every item was re-decided against a running system. **Six fix passes landed that
same day** (`2a31ff3`, `e6a3c7b`, `1bebb7f`, `0f545a9`, `8bde487`, `030a2c3`, `6f06234`) and this
file did not move with them, so by the afternoon it was stale in the reassuring direction — the
dangerous one. It listed as open six things that were already fixed and had tests.

This pass re-decided **every** numbered item and every bullet again. Where the answer could be
reached by running something, it was: a real `kampr serve` on 127.0.0.1:8795 against an isolated
herdr session created for the purpose (the user's `default` was never touched), real HTTP and
WebSocket probes against it, the compiled client's own suites, and the Rust suites against a real
herdr 0.8.2.

Every verdict below carries the command that decided it and says whether it was decided **by
execution** or **by reading**. The rule that governs this file is unchanged: **an item leaves the
list only when the fix has a test proving it** — so where a fix is claimed, the test is named and
was run.

**Amended later the same day.** Two entries below were re-decided after a fix pass that closed both:
**M1** (a refused write leaves no audit line) and the decision section *A mid-session demotion is
announced*. Both were the same defect wearing two hats — a device whose permissions changed, or which
is probing what it can do, was invisible — and both now carry named tests that were written first and
run red. Nothing else in this file moved; the counts in the table above are from the second pass and
have not been re-measured.

## The commands this pass ran

| | |
|---|---|
| `cargo test -p kampr-node --test live` | **37 passed, 0 failed** (see the flake note below) |
| `cargo test -p kampr-node --test {limits,backpressure,push,transfer,android_passkeys,manage_wire,mesh}` | 5 / 4 / 6 / 2 / 10 / 5 / 2, all green |
| `cargo test -p kampr-node --lib` | 57 passed |
| `cargo test --workspace` | kampr-auth 87, kampr-cli 41 + 31 + 2, kampr-core 9 + 2 + 7, kampr-journal 36, kampr-mesh 19 + 7 — all green. `crates/kampr-core/tests/registry.rs` is **in flight** and one of its tests fails against uncommitted work; not decided here |
| `./gradlew :shared:jvmTest :terminal:jvmTest :mosaic:jvmTest` | 139 / 81 / 32, **0 failures** |
| `KAMPR_LIVE=1 KAMPR_URL=… KAMPR_TOKEN=… ./gradlew :shared:jvmTest --tests '…LiveNodeTest.theClientReadsTheAuthSurfaceOfARealNode' --tests '…LiveNodeTest.preferencesWrittenOnOneConnectionComeBackOnTheNext'` | **2 tests, 2.09 s, no skip marker** — the real client against a real node |
| `curl` route matrix and a raw WebSocket client against 127.0.0.1:8795 | see items #6 and the audit-hole bullet |
| `git tag` | **empty** |

**One flake, new, mentioned by neither list.** The first full run of `--test live` at
`--test-threads=2` failed `resync_repaints_every_watched_pane_and_unwatch_stops_one` at
`live.rs:1079` — *"an unwatched pane must stop streaming"*, left 1, right 0. It passed 3/3 in
isolation and did not recur in two further full runs. So it is timing, not a defect in the code
under test — but a suite that fails one time in three under contention is a suite that will be
re-run rather than believed, and this is the only one in the tree that does it.

## The root cause, restated

> The tests were written from the same mental model as the code, so both sides of a seam agree with
> each other and neither agrees with the wire.

Still the right diagnosis, and this pass is the first where the record shows it being **paid off
rather than re-found**. The clearest instance the previous pass named — `workspace.create` working
in `LiveNodeTest` and failing against a real herdr, because the test always passed an `env` map the
client's New sheet never sends — is fixed at the node, and the test that proves it drives all three
shapes rather than the convenient one.

Three structural facts, updated:

- **CI now proves something about the node.** Unchanged: `.github/workflows/ci.yml` installs herdr
  from `herdr.dev/latest.json`, checks the hash, and fails the job if the suite prints
  `skipping: no herdr on PATH`.
- **CI now also proves something about the client against a real node.** This was the single
  loudest finding of the previous pass and it is **closed**. `ci.yml:83-105` starts a herdr, runs
  `kampr init` and `kampr serve`, pairs a device, and runs `LiveNodeTest` with `KAMPR_LIVE=1` — and
  `LiveNodeTest.kt:29-42` turns a silent skip into a failure when that variable is set. Two of the
  three tests run there. **The third, `theClientDrivesARealHerdAndLearnsAboutItFromThePatch`, does
  not** — it needs a herd with panes in it, and CI's herdr is empty. That is the remaining hole, and
  it is the one that would have caught the `env` defect.
- **The roadmap tick state is still worthless.** Unchanged. `docs/02-roadmap.md`'s per-phase table
  is the truth there; the checkboxes are not.

## Broken

Of the nineteen: **all nineteen are now closed**, including the two that carried named residuals
(#4, #19) and the one (#15) that was wrong as written and was replaced by N2 — which is also closed.
Item #6 is closed in a way worth reading, because the node did not change.

| # | Defect | Verdict | What decided it |
|---|---|---|---|
| 1 | Revoking a device does not disconnect it | **FIXED — execution** | `live.rs::revoking_a_device_hangs_up_the_socket_it_is_already_using`, green. The already-open socket receives `error{revoked}` and `CLOSE`, not "whenever the socket happens to drop" |
| 2 | Binds `0.0.0.0` in cleartext by default | **FIXED — execution** | `kampr init --bind 127.0.0.1:8795` on a fresh config dir wrote `bind = "127.0.0.1:8795"`, and the default without `--bind` is `config.rs:161`'s `127.0.0.1:{DEFAULT_PORT}` |
| 3 | A read-only device can promote itself to full | **FIXED — execution** | `live.rs::a_readonly_device_is_refused_writes_and_still_sees_the_pane` and `::a_readonly_device_is_refused_every_manage_op`, green. And on the live node: `kampr pair` with a device already enrolled printed an **unarmed** code and said so — *"a read-only device sees every pane, so this code does nothing until you allow it from this console"* |
| 4 | The conversation server side does not exist | **FIXED, and the residual is closed — execution** | `live.rs::a_watched_agent_pane_streams_its_conversation`, green. The residual was `has_conversation` derived from the *agent kind*: `state.rs:318-321` now derives it from a transcript actually resolving, cached with a 5 s negative floor, and `live.rs::a_freshly_started_agent_claims_no_conversation_until_one_exists` is the test. The blank Conversation pane on a just-started agent is gone |
| 5 | **`herd.patch` is structurally incompatible** | **FIXED — execution, end to end** | Unchanged from the previous pass and re-proved from the other side: `crates/kampr-core/tests/protocol.rs` (7 green) pins the node's shapes, `ProtocolUpdateTest` (8 green) and `ModelTest` (12 green) drive them through the compiled `Wire`/`KamprStore` |
| 6 | **Three of four auth HTTP paths do not exist** | **FIXED — execution, but not where you would look** | The **node still does not route them.** Against the live node: `GET /auth/status` → **200 `text/html`**, `GET /auth/devices` → **200 `text/html`**, `POST /auth/devices/<id>/revoke` → **405**, `POST /auth/pair` → 200. What changed is the client: `AuthApi.kt:62-64` records the three dead paths as removed, `devices()` now calls `/api/devices`, `revoke()` `/api/devices/{id}/revoke`, `pairingCode()` `/api/pair`. The whole seam below it moved too — see the note |
| 7 | `kampr.db-wal` world-readable, unsalted SHA-256 | **FIXED — execution** | On the freshly created state dir: `kampr.db`, `-wal`, `-shm`, `audit.jsonl`, `vapid.pem`, `node.key` all `-rw-------`, dirs `drwx------`. `secret.rs` stretches with Argon2id. **Residual, by design:** the salt is a constant (`kampr/pairing/v1`), because the lookup is by digest |
| 8 | `trust_proxy` reads the leftmost `X-Forwarded-For` | **FIXED — execution** | `live.rs::a_forged_forwarded_for_buys_no_fresh_rate_limit_bucket` and `::a_forged_forwarded_header_does_not_buy_a_fresh_rate_limit_bucket`, both green |
| 9 | The pairing `attempts` counter is read but never written | **FIXED — execution** | `live.rs::a_revoked_token_cannot_reconnect_and_a_wrong_code_never_gets_one`, green; rate-limit half in `tests/limits.rs` (5 green) |
| 10 | **The node errors on unknown `t`** | **FIXED — execution** | `session.rs:36-45` now keeps a `CLIENT_VERBS` list beside the enum and checks the vocabulary *before* handing the value to the tagged enum, so an unknown verb is ignored and a malformed known verb still fails. `live.rs::an_unknown_tag_draws_no_answer_at_all` (sends `{"t":"future.thing"}`, fences with a `ping`, asserts **zero** `error` frames) and `::an_unknown_message_is_ignored_and_a_bad_one_is_refused`, both green. There is also a drift guard, `session.rs:1108::the_verb_list_is_exactly_what_the_enum_decodes` — the two lines that used to contradict each other now have a test between them |
| 11 | `answer` never sends Codex's submit key | **FIXED — execution at the unit, not at the harness** | `session.rs:1090` `SUBMIT_KEYS = [("codex","\r")]`, `:550` extends the keystrokes, and the loop issues them as separate `registry.write` calls so herdr cannot coalesce them into a paste. `session::tests::only_the_harnesses_that_need_a_submit_key_get_one` is green (`cargo test -p kampr-node --lib`, 57 passed) and cites probe #43. **Still not re-driven against a live codex** — probe #79 stands unreconfirmed, three passes running |
| 12 | **`applyReset` appends to the link table** | **FIXED — execution** | `PaneState.kt:106` is now `links.clear()` before `links += msg.links`; `applyPatch` still appends, which is correct for a suffix. `LinkTableTest` (2 green): `aResetCarriesTheWholeTableAndAPatchOnlyTheSuffix` asserts the table is *replaced*, `theLinkOnTheLastRowPrintedResolvesToTheUrlThatRowDeclared` asserts the id resolves. **Stale prose, not code:** the comment at `PaneState.kt:103-105` still says *"Appending here is what makes a post-reset id resolve"* two lines above the `clear()` that replaced it |
| 13 | The same-origin gate is self-satisfying under DNS rebinding | **FIXED — execution** | `live.rs::an_origin_the_attacker_chose_never_satisfies_the_same_origin_gate`, `::a_session_cookie_is_not_a_credential`, `::the_http_surface_refuses_what_it_should`, all green |
| 14 | `readonly` is not gated on `prefs` or `caps` | **FIXED — execution** | `live.rs::prefs_and_caps_are_bounded_rather_than_an_amplifier`, green. Bounded at `MAX_PREFS_BYTES = 2048` for every role rather than gated, as the fix claimed |
| 15 | `manage.rs` drops the `Child` — a zombie per session | **WRONG AS WRITTEN — replaced by N2, which is now fixed** | The previous pass proved there is no zombie (`ps -eo stat --ppid <node>`: three `Sl`, zero `Z`; tokio's orphan queue reaps them). The real defect it was hiding is N2 below, and N2 is closed |
| 16 | `kampr serve` needs herdr before it binds | **FIXED — execution** | `live.rs::the_port_is_bound_before_herdr_is_needed`, green. Re-seen by hand: a node pointed at a herdr socket with nothing behind it still answered `/healthz` and `/` |
| 17 | A herd outage was invisible | **FIXED — execution** | `live.rs::a_herdr_outage_reaches_the_client_and_recovers`, green |
| 18 | Only one herdr session was ever served | **FIXED — execution** | `live.rs::every_herdr_session_on_the_host_is_its_own_node`, green. Seen by hand too: with `[herdr] sessions` absent the live node listed every session on this host as its own node with its own `rtt_ms`; with `sessions = []` it served exactly the one its socket resolves to. Both states are expressible, which is the point of `Option<Vec<String>>` (`config.rs:121-127`) |
| 19 | Panes observed at the layout rect | **FIXED, and the residual is closed — execution** | The grid was already correct. The residual was `herd.panes[].cols` reporting the rect for an *unwatched* pane: `PaneEntry.cols` is now `Option<u16>` with `skip_serializing_if`, `known_cols`' rect fallback survives only where a width is mandatory (re-wrapping the ring), and `proven_cols` returns nothing rather than a lie. `live.rs::an_unmeasured_pane_reports_no_width_rather_than_its_rect` and `::a_split_pane_is_observed_at_the_pty_width_not_the_rect`, both green. The three client sites that printed the number now print `—` (`PaneScreen.kt:228`, `MosaicCell.kt:118`) |

### What #6 actually was, and what fixing it took

The node never grew `/auth/status`, `/auth/devices` or `/auth/devices/<id>/revoke`, and it should
not: the device inventory has always been at `/api/devices`. **The client was calling paths that had
never existed**, and the SPA fallback answered two of them with HTML, so the failure looked like an
empty device list rather than a 404.

Repointing the URLs was the smallest part of it. Three more layers of the same seam had to move, and
all three did:

- `DeviceRecord` declared `@SerialName("added_at")` / `("last_seen")`; the node sends `created_at` /
  `last_seen_at` — confirmed against the live node, whose `/api/devices` body reads
  `{"created_at":…,"expires_at":…,"last_seen_at":…,"origin":…,"revoked_at":null,"role":"full"}`.
  `AuthApi.kt:33-47` now matches it, as `Long?` epoch seconds.
- **No endpoint served `SetupStatus`'s shape at all**, and none does now. `AuthApi.kt:16-27` says so
  in its own header — *"Composed, not fetched: no route serves this shape"* — and builds it from
  `/api/node` plus `/api/devices`. `address` stays non-optional because it can no longer be missing.
- The revoke result was computed and thrown away. `KamprApp.kt:135-139` now sets `authFailure` when
  `revoke` returns false, and that renders as an `ErrorStrip`.

`LiveNodeTest.theClientReadsTheAuthSurfaceOfARealNode` is the test, it ran against a real node in
this pass, and it runs in CI. **The one thing still not covered by a hermetic test** is the UI branch
that sets `authFailure` — that assertion lives only in the live test.

## Newly found in the previous pass — all re-decided

| # | Defect | Verdict | What decided it |
|---|---|---|---|
| N1 | **`workspace.create` fails unless the client sends an `env` map** | **FIXED — execution** | `manage.rs:302-315`'s `env_map` returns `None` for absent, `null` *and* empty, and `manage.rs:102-108` only sets the key when there is one — because herdr 0.8.2 types `env` as a map and refuses a `null`. `live.rs::a_workspace_with_no_environment_is_created` drives all three shapes (no key, `{}`, `{"KAMPR_LIVE":"1"}`) against a real herd, green. This is the defect the New-workspace button hit every time |
| N2 | **A created herdr session shares the node's cgroup, and the unit sets no `KillMode`** | **FIXED — execution** | `packaging/kampr.service:18` is `KillMode=process`, with the measurement in the comment above it; `PrivateTmp` is deliberately unset and the comment says why. `crates/kampr-cli/src/service.rs:403` `include_str!`s that one file rather than keeping a second copy. Tests: `service.rs::the_unit_signals_the_node_and_not_the_sessions_it_created` (asserts both the `KillMode` line and the absence of `PrivateTmp`) and `live.rs::a_created_session_leaves_the_nodes_process_group` — the other half is `manage.rs:271` `command.process_group(0)`. Both green |
| N3 | **The client decodes `scrollback.capped` / `complete` and no UI reads either** | **FIXED — execution** | Three surfaces read them now: `HistoryEdgeMark` (`view/HistoryEdge.kt:23-40`, an 18 dp strip at the top of the scroll surface, mounted from `TerminalView.kt:395-407` with layout space reserved at `:218`), the `ReviewStrip` tint and its spoken text, and `ZoomSheet`'s `historyNote`. The classifier is `Review.kt:34-39` — `None / Whole / Clipped / Discarded`. Tests: `GridAccessibilityTest.ScrollbackHonestyTest` (3), `ReviewTest.theTopOfTheSurfaceSaysWhereTheRecordEndsAndWhy` / `onlyAnIncompleteRecordWarns` / `aDiscardedRowTellsTheReaderRatherThanQuietlyRelocatingThem`. All green in the 81-test `:terminal:jvmTest` run |
| N4 | **After a burst outruns the poll the node's history is permanently ~1000 rows** | **OPEN, and not Kampr's to close** | Unchanged and re-confirmed by reading: `pane.read recent` caps at 1000 and takes no offset (probe #51), so nothing can backfill a discard. What *has* changed is that the node says so on the wire and the client now shows it (N3). This is upstream ask **U8c**, not a defect list item — it stays here only so nobody re-discovers it as a bug |
| N5 | `session.create` / `session.stop` return a `managed.id` shaped like a pane id | **FIXED — execution** | `manage.rs:274`/`:291` return `{"session": name}` and `session.rs:629-634` branches on that before node-qualifying anything; `created_id` deliberately never matches `session`. `live.rs::a_created_session_leaves_the_nodes_process_group` asserts `stopped["id"] == session`, and there is a unit test that `created_id(json!({"session":"agents"}))` is `None`. Green |

## Newly found this pass

| # | Defect | Verdict | What decided it |
|---|---|---|---|
| M1 | **A refused write leaves no audit line.** `may_write` emitted `error{not_writer}` and recorded nothing, and the `manage` path audited *after* the role gate, so a refused op was unaudited too. The same gate on the HTTP surface — the device inventory, the pairing and mesh surfaces, passkey registration — returned 403 and wrote nothing either | **FIXED — execution** | Every one of those paths now records a `refused` line carrying the verb, the pane, the device, its role, the peer and the error code, bounded by `kampr_auth::audit::Refusals` so a retry loop costs a line per doubling (attempts 1, 2, 4, 8 …) rather than a line per attempt, with a quiet minute starting a fresh incident. Test: `live.rs::a_refused_write_is_audited_and_a_retry_loop_does_not_flood_the_log` drives a read-only device through `input`, `answer`, `notify`, `manage` and `GET /api/devices` on a **live node** and asserts on the real `audit.jsonl`, then sends 41 refused inputs and asserts both that the log did not flood and that the count survived. Written first and run red against the old code — it reproduced probe #125 exactly, the log holding only `pairing.*` and `session.opened`. Plus four unit tests on the bound in `kampr-auth::audit` |
| M2 | **The outermost mosaic cell's close button can fall under a side navigation bar.** `MosaicCell.kt` never reads `LocalSafeArea` at all — its header `Row`'s only padding is `start = 4.dp, end = 4.dp` — and its parent `MosaicGrid` applies no horizontal inset either. (`MosaicCell.kt` is under active edit, so this cites the composable rather than a line.) On a tablet in landscape with three-button navigation, the right-hand cell's cross sits under the bar | **OPEN — reading, and a test gap** | The switcher's own close button *is* inset — its bar `Row` carries `absolutePadding(left = safe.left, …, right = safe.right)` in both orientations — and `MosaicScreen.kt`'s bar and status row read `LocalSafeArea` too. The cell does not. `MosaicSafeAreaTest`'s only fixture is `SafeArea(top = 32.dp, bottom = 46.dp)` — `left` and `right` are never set, so the case cannot fail there. Side bars *are* exercised elsewhere (`SafeAreaTest.SIDE_BARS`, `KeyRowSafeAreaTest`), which is what makes the mosaic's omission a gap rather than a policy |
| M3 | `KamprConnection.heartbeat()` still discards the result of `outbox.trySend(ClientMsg.Ping(n))` | **OPEN — reading, and harmless** | Every keystroke path now checks it (see the flaky-link bullet). A dropped ping costs a heartbeat, not a character. Recorded so the next reader does not think it was missed |

## Incomplete

- ~~**Per-device prefs never restore.**~~ — **CLOSED — execution.** `session.rs:179-195` sends a
  `prefs` frame right after `hello` and `herd`; the live socket this pass opened received
  `hello, herd, prefs` in that order. The destructive one-key write is gone too: `session.rs:743-786`
  merges key by key and treats `null` as "clear this one", with the 2 KiB bound applied to the merged
  result. `live.rs::prefs_are_restored_at_hello_and_merged_on_a_partial_write` and
  `::pane_preferences_are_stored_per_device`, both green, and
  `LiveNodeTest.preferencesWrittenOnOneConnectionComeBackOnTheNext` ran against a real node in this
  pass and in CI. The client sends one key per write, which is now the correct shape rather than the
  lossy one.
- **PWA.** Unchanged and still accurate as re-stated last pass: `manifest.webmanifest` and a 224-line
  `sw.js` both ship, the worker deliberately does not proxy the app's own fetches, there is **no
  install prompt** and **no offline shell, by choice**.
- **The install path is built and has never run.** **STILL OPEN — execution.** `git tag` is empty.
  `release.yml` is tag-triggered (`on: push: tags: ['v*']`) and pins cosign's certificate identity
  to `release.yml@refs/tags/…` (`:197`), so a `workflow_dispatch` run builds and smoke-tests but
  never signs or publishes — by construction, and the file says so in its own header. **Nothing has
  ever been cross-compiled for macOS, signed with cosign, or published with `gh release create`, and
  `install.sh` and `herdr plugin install` have nothing to fetch.** aarch64 Linux was proven by hand;
  the release path around it was not.
- **`min_herdr_version` is enforced by `kampr doctor` and nowhere else.** **STILL OPEN — execution.**
  `MIN_HERDR_VERSION` appears only in `crates/kampr-cli/src/doctor/herd.rs` (and a unit test there
  asserting `herdr-plugin.toml` declares the same floor). `kampr-node` never compares a version to
  anything; it only reports `herdr_version` into the herd. **The node starts against any herdr**, and
  a version skew is silent unless someone runs `doctor`.
- **No protocol-version negotiation.** **STILL OPEN — reading.** `Codec.kt:32` decodes
  `hello.protocol` into `ServerMsg.Hello.protocol` and there is not one comparison of it anywhere in
  the client. Note this is *not* the same as unknown-`t` tolerance, which is now fixed (#10): the node
  will survive a newer client's vocabulary, and neither side will ever notice a version gap.
- **Accessibility.** **Partly closed, and re-measured from the source rather than an emulator.** The
  terminal now *has* a review mode (`terminal/review/Review.kt`, `ReviewState.kt`, `ReviewStrip.kt`,
  `ReviewTest`), so the roadmap's "no review mode" is stale. Touch targets: `TOUCH = 44.dp` and
  `LANDSCAPE_TOUCH = 36.dp` are named constants (`Accessibility.kt:40-41`), key caps are
  `defaultMinSize(minHeight = 44.dp)` with `GridAccessibilityTest:153` asserting `≥ 44.dp`, and
  `HerdAccessibilityTest:166-167` asserts both axes on the herd rows. **Two controls are still under
  44 dp:** the column indicator's control at `defaultMinSize(minHeight = 26.dp)`
  (`ColumnIndicator.kt:93`) and the `Segmented` control at `LANDSCAPE_TOUCH` — the second deliberate,
  the first apparently not. **No emulator was run this pass either**; these are source measurements.
- **Warm resume is per-process only.** **STILL OPEN — reading.** Nothing persists a grid, a herd or a
  transcript across a process restart. The only things written to disk go through `Prefs`: recent
  addresses, device id / endpoint / token, theme, mode, agent args, and the mosaic arrangement.
  `AppState.warm()` is an HTTP re-fetch, not a cache read.
- ~~**Silent failure on a flaky link.**~~ — **CLOSED — execution.** `KamprConnection.send()` now
  checks `trySend(...).isSuccess` and calls `store.noteInput(pane, delivered)`; `PaneState.undelivered`
  counts what was dropped and the UI reads it; `discardTyping()` drains keystrokes on teardown and
  re-queues only standing intents. `OutboxTest` (2 green) types 200 keys at a dead socket and asserts
  all 200 were signalled and none replayed. The other half is closed too: `AppState.kt:256-269` no
  longer persists the pairing code as a device token — a refused pair sets `pairingError` and returns
  null. **Residual: M3.**
- **Observability.** **STILL OPEN — execution.** `http.rs:88` is
  `.route("/healthz", get(|| async { "ok" }))` and the live node returns the literal string `ok`.
  There is no metrics endpoint anywhere in the router.
- ~~**Audit log holes.**~~ — **CLOSED, with one new hole: M1.** Everything the original bullet named
  is filled — failed auth, wrong codes, rate limiting, `watch` with its flags, `manage` with `cwd`,
  `env`, `label`, `node`, `kind`, `name`, and rotation with a test. What is *not* recorded is a
  refusal; see M1.
- ~~Missing outright: destructive-command confirm, threat model, ARCHITECTURE.md, ADRs.~~ —
  **ALL FOUR EXIST.** `guard/Destructive.kt`, `SubmitGuard.kt`, `ConfirmSheet.kt`, `DestructiveTest.kt`;
  `docs/08-threat-model.md`; `ARCHITECTURE.md` (now committed); ten ADRs under `docs/adr/`.
- **Mesh.** `crates/kampr-node/tests/mesh.rs` (2) and `crates/kampr-mesh/tests/relay.rs` (7) are green
  on one host. What one machine still cannot prove is unchanged: real latency, a real NAT, a real
  reverse proxy. The client half of the latency indicator is drawn.
- **Notifications.** Unchanged and still true: `tests/push.rs` (6) is green against an RFC 8030 stub,
  and **no physical device has run any of it**. The iOS Add-to-Home-Screen detection is written
  against documentation. **Android push additionally needs the user to install a UnifiedPush
  distributor** — `PushPlatform.android.kt:104-108` reports `NeedsDistributor` when
  `UnifiedPush.getDistributors()` is empty and `NotificationsScreen.kt:262-267` blocks the screen with
  an explanation. There is no install affordance and no test of that branch.
- **A passkey has never been created, anywhere.** **STILL OPEN — execution, probe #116.** A stock AVD
  with no Google account has no credential provider at all; GMS answers `No create options available`.
  Everything up to the provider is verified — `tests/android_passkeys.rs` (10 green) covers the asset
  links document, fingerprint canonicalisation and the ceremony options, and
  `PasskeyAndScannerTest` checks that an authenticator is offered only once there is an Activity to
  raise it from and that the app knows its own signing certificate. **None of it creates a credential.**
  Run it once against a real phone.
- **The QR scanner's camera path has no automated test.** **STILL OPEN — reading, probe #117.** Every
  test stops at `decodeQrLuminance(luma, rowStride, width, height)`: `QrScanTest` builds a synthetic
  luminance plane with a configurable stride pad, `QrDecodeTest` round-trips the encoder,
  `PairingScanTest` covers URL parsing, and the instrumented test does the synthetic frame again.
  `decodeQr(ImageProxy)` and the `ImageAnalysis.Analyzer` around it are `private` in
  `PairingScan.android.kt` and nothing drives them — so the plane extraction, the one-shot
  `compareAndSet` latch and the `view.post` hand-off are unexercised. The emulator cannot help
  (probe #117); this needs a phone or a seam.

## In flight — not decided by this pass

Two areas are being worked on right now by other agents. They are recorded here so nobody reads their
absence as "fine", and deliberately **not** described as either open or fixed, because the answer is
being written as this is:

- ~~**The 3-second herd poll runs with no clients connected**~~ — **FIXED.** Event-driven, with a
  sweep behind it at 30 s idle and 3 s for as long as any pane is streamed. The fast cadence stays
  because a desk resize emits no event at all (#52), so the sweep is the only thing that sees one.
  Measured: **~40 herdr calls a minute at idle down to 4**, change-to-client latency unchanged
  (198.5 ms → 189.6 ms). Stated trade: on a node nobody is watching, a resize takes up to 30 s to
  reach the herd list.
- ~~**Watcher presence is not on the wire**~~ — **FIXED.** `herd.panes[].watchers`, omitted at 0 and
  1, with the omit rule in one place so the node and the wire doc cannot drift. It **under-counts by
  design** and never over-counts: the desk operator is not a Kampr viewer, everyone behind a hub
  counts as one, and a client that has not watched yet counts as none. The client renders "at least
  N" for exactly that reason, and says *watching, not necessarily typing*.
- ~~**Probe #112** — the unexplained repeated `pane re-wrapped; ring restarted`~~ — **DIAGNOSED AND
  FIXED, and it was a real defect**, not the ordinary trimming it resembled. `ScrollbackRing::restart`
  adopted the new rows but kept the **old** `cols`, so once a soft wrap proved the PTY a column
  narrower than its rect (#69), every subsequent read disagreed with the ring for ever: a restart per
  read, `from_top` climbing ~285 rows/s on a pane that had gone quiet, history pinned at one read's
  depth, `capped` stuck true, and the whole ring re-sent to every client every 3 s.

## The three items the file flagged as decisions

### A mid-session demotion is announced — **decided, built and tested**

**Enforcement was already proved.** `live.rs::a_demotion_and_an_expiry_both_land_on_the_open_socket`
(green) demotes a device straight in the store while its socket is open, and the very next `input`
comes back `error{not_writer}`; an expiry set past `now()` closes the socket within 15 s. `refresh`
picks the change up on the 2 s `DEVICE_RECHECK`.

**The announcement now exists.** The decision was a **new server → client message**, not a re-sent
`hello`: `04-wire-protocol.md` defines `hello` as the *first* message on a connection, and quietly
making it re-sendable would break that contract for every future reader, while a dedicated `t` is
additive and the node already ignores unknown `t` values in both directions.

```jsonc
{ "t": "role", "role": "readonly" }        // "full" | "readonly", the same two `hello` uses
```

Sent from `Session::refresh` whenever the effective role moves, in **both** directions — a promotion
is the same problem in reverse, and a device upgraded to full gains its affordances without
reconnecting. Documented under
[`role`](./04-wire-protocol.md#role--this-devices-role-changed-mid-connection).

On the client, `KamprStore.readOnly` is no longer `_hello.value?.role`: the store holds `role` as
Compose snapshot state, so `readOnly`, `canManage` and every surface gating on them recompose when it
moves — the key row, the New sheet, the pane manage actions, the pending bar, the read-only badge.
`hello` is left exactly as the connection was greeted. A change nobody announced is one that gets
discovered by pressing something that no longer works, so the store also raises `roleNote`, drawn
through the same dismissible strip the passkey note uses and announced as a **polite** live region —
the vocabulary `Accessibility.kt` already reserves urgent for a blocked agent and a command about to
run, and this is neither. `ErrorStrip` and `NoteStrip` moved out of `KamprApp.kt` into `ui/Strips.kt`
so the notice reuses them rather than growing a second one.

Tests, both written first and run red:
`live.rs::a_demotion_and_a_promotion_are_both_announced_on_the_open_socket` asserts the frames on a
real socket, that no `hello` precedes either of them, and that the promotion is real and not just
announced; `RoleChangeTest.aRoleChangeOnALiveSocketReachesTheStore` pushes the same bytes down a real
WebSocket into the real store, and `…theWriteAffordancesFollowTheRoleWithoutAReconnect` renders
`PaneScreenMobile` and asserts the New action leaves and the read-only badge and the spoken notice
arrive. Non-vacuity was measured three ways: with the codec case removed both client tests fail; with
`readOnly` restored to `_hello.value?.role` both fail; and with `role` held as a plain field rather
than snapshot state the socket test passes and the affordance test still fails — which is the whole
reason the store uses snapshot state and not a `StateFlow`.

`KamprStore.kt`'s own comment — *"the affordances must be absent, not present-and-failing"* — now has
a wire that lets it hold.

### The VT-emulator payoff is still not collected — **confirmed, by reading**

`kampr-term` exposes `new`, `feed`, `reset`, `resize`, `grid`, `cursor`, `take_dirty`. Selection is in
the client, link resolution is `PaneState.links` plus `CellBuffer.linkAt`, soft-wrap joining is in the
client, and **there is still no find over the grid on either side**. The only search in the tree is
`conversation/Search.kt`, over the transcript. Review-mode stepping (`ReviewMove.PreviousWord`, …) is
navigation, not search. Either collect the benefit or stop claiming it. Unchanged.

### The gap sentinel — **the cheap first move was taken**

The arithmetic correction from the previous pass stands and is not repeated here: there are two polls
and only the registry's adaptive one races herdr's cap; the measured threshold is between 1 200 and
2 500 rows/s, not 330; `cargo build` on this workspace is ~2.6 rows/s, three orders of magnitude under
it. The failure is still easy to reach — `seq 1 50000`, any `find /`, any `cat` of a large file — and
still permanent, because `pane.read` has no offset.

What has changed is the recommendation, which was *"wire the `capped` flag into the UI before adding
a sentinel"* — **that is done** (N3). The history edge now says which of three things happened, in a
strip, in the review strip's speech, and in the zoom sheet. So the remaining decision is only whether
a sentinel row in the ring itself is worth anything beyond that, and the honest answer is: not until
someone has lived with the strip and found it insufficient.

## Planned and never built — corrected

Checked against code on 2026-08-21, second pass.

| Item | Status |
|---|---|
| Destructive-command confirm (P2.10) | **Built** — `guard/Destructive.kt`, `SubmitGuard.kt`, `ConfirmSheet.kt`, `DestructiveTest.kt` |
| Recovery code (P3.5) | **Done** — `kampr recover`, ~99 bits, single use, argon2id digest under its own salt domain. Seen printed by `kampr init` this pass |
| `kampr doctor` (P7.7) | **Done** — 14 checks on this machine (`kampr doctor --json`), `--json`, non-zero exit on a real failure. The asset-links origin check landed in `6f06234` |
| Light theme / `prefers-color-scheme` (P6.8b) | **Built** |
| Per-theme ANSI palette (P6.8c) | **Built** ([ADR 0009](./adr/0009-the-terminal-keeps-its-own-ground.md)) |
| Accessibility (P6.11) | **Built**, including a review mode ([ADR 0010](./adr/0010-the-grid-is-described-not-read-out.md)), with the two under-44 dp controls listed above |
| PWA manifest + service worker (P6.10) | **Built**. No install prompt; no offline shell, deliberately |
| Kampr split view / mosaic (P4.5.8/P4.5.9) | **Built** — `client/mosaic`, 32 green tests. Safe-area gap M2 |
| Mesh latency indicator (P4.9) | **Built** — drawn in four places |
| Kampr on Android as a *provider* (Phase 8.5) | **Cut** 2026-08-21 |
| ARCHITECTURE.md, ADRs, threat model (P3.1, P9.2, P9.3) | **Written** |
| Terminal find | **Absent on both sides** |
| Release tag | **Never pushed** — `git tag` is empty |

## Verified good — re-checked

- `kampr-term` matches Herdr's grid exactly.
- The `Outbox` backpressure design, including the `scrollback`/`styles` purge exemption
  (`tests/backpressure.rs`, 4 green).
- The scrollback stitching and its gap reporting — honest on the wire **and now read by the client**.
- `ModeSelector` implements probe #62 with hysteresis.
- WebAuthn ceremony hygiene, and the Android asset-links work around it (`tests/android_passkeys.rs`,
  10 green) — everything short of an actual credential.
- JSONL audit injection is impossible and the 0600 is real (re-verified on a fresh state dir). The log
  covers failed auth, watch, full manage detail and — since M1 — every role refusal on both the socket
  and the HTTP surface, and rotates.
- The same-origin gate (#13), the revocation path (#1) and the herd-outage path (#17) behave under
  attacker-shaped probes, not just under their tests.
- **New to this list:** the read-only role holds on every verb tried against a live socket, and the
  refusal now arrives as a `managed{ok:false}` **ack as well as** an `error` — a client that clears
  in-flight state on the ack no longer hangs on a refusal.
- The probe log is still the best artefact in the repo.

## What would have caught what

The test the original audit asked for — start the real node, connect the real client codec, assert on
bytes — exists, runs in CI, and **fails loudly rather than skipping** when it is meant to be live. Of
the three things this file has ever blamed on its absence, two are now covered by it.

One gap is left, and it is the same shape as the defect that motivated it: CI runs `LiveNodeTest`
against an **empty** herd, so the third test — the one that drives `manage` ops the way the client's
own screens drive them — does not run there. `every_client_op_lands_on_a_real_herd` covers the node
side of that against a real herdr, but it is the node's own idea of what a client sends. **Give CI's
herdr a workspace and run the third test**, and the loop this file was written about is closed.
