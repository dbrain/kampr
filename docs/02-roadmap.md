# Kampr — roadmap

Tick as you go. Derived from `01-implementation-findings.md`; every item traces to a finding there.

> **Tick state is unreliable and was never maintained during the build.** Treat the per-phase
> summaries below as the truth and `docs/06-audit.md` as the live gap list; the individual checkboxes
> lag reality in both directions. Five ids were duplicated (P1.3, P1.4b, P1.6c, P4.5, P8.5) — a
> checkbox is a note to a human here, not an identifier to cite.

## Where each phase actually stands

| Phase | State |
|---|---|
| **Phase 1** | ✅ Streaming, provider seam, pane registry, VT emulation, wire encoding. Verified against a live herdr; `kampr-spike` reproduces herdr's own grid exactly. |
| **Phase 2** | ✅ Renderer, both render modes, zoom and pan, key row with the inverted-T cluster, live input, selection, links, paste framing, destructive guard. |
| **Phase 3** | ✅ Tiered auth, devices, roles, passkeys server-side, audit log, recovery code. Nine security defects found by audit and closed with tests. |
| **Phase 4** | ✅ Mesh: peers dial out to a hub, ed25519 mutual auth, relay with per-hop backpressure. Proven across two nodes on one host. |
| **Phase 5** | ✅ Conversation view, both halves. Claude and Codex adapters, markdown with real tables, turn revision by id. |
| **Phase 7** | ✅ Setup ladder, plugin manifest, service supervision, verified install path, `kampr doctor`. |
| **Phase 6** | ⚠️ Responsive layouts, themes and the light ground are done. **Accessibility (P6.11) is not started** — zero `semantics`/`contentDescription` in the client. **PWA (P6.10) is not built**, yet `security.installable` advertises it. |
| **Phase 8** | ✅ **Built.** Per-pane status subscription (mean 2.33 s faster than the poll, probe #78), VAPID, service worker, warm prefetch, batching, the question in the body, deep link, snooze and mute, triage list. Proved against a real Firefox and Mozilla's push service. P8.6/P8.7 are the remainder. `docs/08-notifications.md` |
| **Phase 8.5** | ⚠️ **Not started.** Kampr as an Android *provider* — distinct from the Android client, which ships. |
| **Phase 9** | ⚠️ Release workflow written and never run: no tag has been pushed, so aarch64 cross-compilation and cosign signing are untested. |
| **Phase 4.5** | Server complete — all 13 `manage` ops. Client in flight. |

**Legend:** `[ ]` todo · `[x]` done · `[~]` in progress · `[!]` blocked · `[-]` cut
**Gate** = do not start the next phase until this is true.

---

## Phase 0 — Close the probe gaps (½ day)

The seven open questions in findings §6. Cheap, and each one can invalidate a Phase 2–4 decision.

- [x] P0.1 **OSC 8 survives `terminal.frame`** — the spike interned a link from the frame stream. `pane.read` drops it, so frames are strictly richer
- [ ] P0.2 `terminal.closed` reason + latency when a streamed pane exits → decides reconnect UX
- [ ] P0.3 Does `observe`/`control` accept an agent-name target as well as `wN:pN`?
- [x] P0.4 **Scrollback answered without `terminal.scroll`** — frames carry end state only; the ring comes from `pane.read recent` (instant, colour-preserving, viewport unmoved) and alt-screen panes have no ring at all
- [x] P0.5 Version floor pinned to **0.8.2** — the only version everything here is verified on. Lower it later with evidence, never with optimism
- [ ] P0.6 Frame-stream cost: CPU/RSS of N concurrent `observe` children on one host → sizes the mux
- [ ] P0.7 Behaviour when the *same* pane is observed at two different geometries simultaneously (already seen working; confirm no cross-talk under load)
- [x] P0.9 **Is a control-mode resize reversible?** Yes — but moot: `control` always claims geometry, so Kampr does not use it
- [x] P0.10 **Can Kampr tell whether a desktop client is attached?** No — and now moot, since nothing is ever borrowed
- [x] P0.11 **Can input be sent without claiming geometry?** Yes — `pane.send_text` / `pane.send_keys` over JSON-RPC. `control` cannot: no-flag control still takes the PTY to 120×40
- [x] P0.13 **`observe` at native size is pixel-exact** — `kampr-spike` matches `pane.read visible` 30/30 rows, cursor included
- [x] P0.14 **`pane.read recent` on a shell pane is safe** — 0.002 s, viewport unmoved, 256-colour intact
- [ ] P0.15 **Is a pending tool request in the transcript before approval?** Decides the answer strip's source
- [x] P0.16 **CMP rendering spike: viable.** 74×30 at 60 fps, 0 dropped, on real WebGL2 and on Android. Two conditions attached — see P2.2c/P2.2d
- [ ] P0.17 Re-measure on a real ARM phone; the emulator flatters the shaping cost
- [x] P0.8 `docs/03-probe-log.md` written — 41 probes, each traced to its command

**Gate:** `docs/03-probe-log.md` exists and P0.1–P0.5 are answered.

---

## Phase 1 — Skeleton that streams one pane (2–3 days)

The riskiest thing in the whole design is a child-process frame stream feeding a browser terminal.
Prove it before building anything around it.

- [x] P1.1 Cargo workspace: `crates/kampr-herdr`, `crates/kampr-term`, `crates/kampr-spike`
- [x] P1.3 `kampr-herdr`: socket RPC, snapshot model, native geometry, observe supervisor, input, scrollback interlock
- [x] P1.6c **Server-side VT emulation working and verified** — `kampr-term`, 9 tests, exact match against Herdr's grid
- [x] P1.4b **Wire protocol written** — `docs/04-wire-protocol.md`
- [ ] P1.2 Rust + axum server, Kotlin/CMP clients; pin the versions in findings §5.5
- [ ] P1.2b Android assets need a staged-assets task — `compose.components.resources` silently omits them for a KMP-library target (probe #64), which will hit tokens, fonts and icons
- [ ] P1.2c Gate first paint on font resolution; use the no-ligature font cut (probes #65, #66)
- [ ] P1.2d Theme token layer first, not last: every colour, font, radius and border through a CSS custom property, `data-theme` on the root, `soft` shipping and `phosphor` / `warm` / `brutalist` defined alongside it (see `docs/design/build.py`)
- [ ] P1.3 `herdr-client` adapter module: the **only** file that knows socket method names or CLI argv
- [ ] P1.3b **Provider seam** — a node talks to `listPanes / streamPane / writePane`, and Herdr is one implementation. Cheap now, expensive later; it is what makes an Android local-PTY node possible at all (findings §3.10)
- [ ] P1.4 Socket dialer (AF_UNIX now; keep the Windows named-pipe seam Collie documents)
- [ ] P1.4b **Wire protocol spec, written before anything is built against it** — cell-grid frames, herd model, input, auth envelope. This is the contract every parallel workstream depends on
- [ ] P1.5 `session.snapshot` → internal herd model, with the `agent_session`-vs-`agent` staleness check from findings §1.10
- [ ] P1.6 Stream supervisor: spawn `herdr terminal session observe <pane> --cols <native> --rows <native>`, parse NDJSON, decode base64, handle `terminal.closed`, restart with backoff
- [ ] P1.6b Native geometry from `layouts[].panes[].rect`; subscribe `layout.updated` and restart the observer when the desk resizes
- [ ] P1.6c **Server-side VT emulation** (`alacritty_terminal`): one emulator per pane, shared by all viewers; emit cell-grid diffs, never raw ANSI, to clients
- [ ] P1.7 WebSocket fan-out to browsers; first subscriber gets the last `full` frame replayed, then diffs
- [ ] P1.8 CMP client boots, draws the cell grid for one hard-coded pane live on wasm and Android
- [ ] P1.8b **Warm resume** — cache the last full frame, render it immediately marked stale, swap on the fresh `full` frame. No spinner (findings §3.11)
- [ ] P1.9 Backpressure: Collie chose polling partly to avoid this. We chose streaming, so we own it — watch `bufferedAmount`, drop to "resync with a full frame" rather than queueing
- [ ] P1.10 Latency budget check on a real phone over the network: is it still SSH-feel?

**Gate:** a phone renders a live Claude pane, and killing/restarting the Herdr server recovers without a browser reload.

---

## Phase 2 — Input, live keys, and the key row (3–4 days)

- [ ] P2.1 Input via `pane.send_text` / `pane.send_keys` only — stateless, no ownership, never touches geometry
- [ ] P2.2 **Zoom and pan** over the native grid: pinch, momentum pan, column indicator, follow-cursor
- [ ] P2.2c **Layer-scale during a pinch, re-shape on settle** — re-shaping at intermediate zooms collapses the run cache and drops frames (probe #60)
- [ ] P2.2d **Two render modes, switched on cache hit rate** — cached run layouts by default, per-glyph `drawText` when a frame's hit rate collapses (probe #59, #62). Do not hand-roll a glyph atlas: it breaks skiko (probe #61)
- [ ] P2.2b Zoom presets (fit width / readable / close up) and per-pane per-device persistence
- [ ] P2.3 Keystroke coalescing per animation frame, ordered, with an in-flight cap
- [ ] P2.5 Raw-escape key table — Esc, Tab, arrows, Home, End, PgUp, PgDn, Del, Ins, F1–F12 — as **bytes**, never as `send_keys` names (findings §1.4)
- [ ] P2.6 JuiceSSH-style key row: two rows above the native keyboard, latching Ctrl / Alt / Shift / Fn modifiers
- [ ] P2.7 Long-press on a key row button → alternates (e.g. Ctrl long-press → Ctrl-C/D/Z/L/R shortcuts)
- [ ] P2.8 Native soft keyboard for text so dictation keeps working; visualViewport handling so the row sits on the keyboard, not under it
- [ ] P2.9 Paste: bracketed paste framing done by us, since `pane.send_text` writes raw bytes with no framing
- [x] P2.10 Destructive-command confirm — hooks **Enter**, not the keystrokes: reads the cursor's logical line off the grid, strips the prompt, and holds the submit. Shell panes only; a paste that carries its own newline is inspected too
- [ ] P2.11 Scrollback: **not available in the live view** (control-mode only). Agent panes use the Conversation view; shell panes use `pane.read recent` if P0.14 says it is safe
- [ ] P2.12 **Hidden-input capture** — offscreen contenteditable read via `beforeinput`/`input` and diffed, because Android soft keyboards do not give usable `keydown` (findings §3.9)
- [ ] P2.13 IME / predictive text: `composition*` handling, `autocapitalize/autocorrect/autocomplete/spellcheck` all off
- [ ] P2.14 Key row docked to the OSK via `visualViewport`, never replacing it — dictation and swipe keep working
- [ ] P2.15 Remote echo end to end: characters appear because the pane echoed them. No compose box, no Send button
- [ ] P2.16 *(optional, later)* mosh-style predictive local echo for slow cellular links

**Gate:** you can drive an interactive TUI (vim, `less`, a Claude permission dialog) from a phone with no desktop client attached.

---

## Phase 3 — Auth, tiered (4–5 days) — *before anything is exposed*

Herdr has no auth at all; the socket is uid-scoped and that is the whole model. Everything here is
ours, and it cannot be one thing — a WebAuthn RP ID must be a domain, so Tier 0 needs its own answer
(findings §3.2, §3.7).

- [ ] P3.1 Threat model doc — state plainly: Kampr access = unrestricted RCE on every meshed host
- [ ] P3.2 **Tier 0 auth**: pairing code → device-bound bearer token, LAN bind, persistent "unencrypted" banner, deliberate-decision expiry
- [ ] P3.3 **Tier 1+ auth**: WebAuthn passkey registration + login, single-user
- [ ] P3.3b Configurable RP ID / canonical origin, with a clear warning when it changes (passkeys are origin-bound)
- [ ] P3.3c Capability detection — hide passkey and notification affordances where they cannot work, and say what a hostname would unlock
- [ ] P3.4 Device enrolment → long-lived revocable per-device token; device list UI with kill switch
- [x] P3.5 Recovery code, generated once, shown once — ~99 bits, argon2id digest, single use, redemption enrols a full device and mints the replacement; `kampr recover`
- [ ] P3.6 Roles: `full` vs `read-only` per device
- [ ] P3.7 Rate limiting + lockout on auth endpoints
- [ ] P3.8 Audit log (JSONL, 0600) of every write action, per device (Collie's `audit.ts` shape)
- [ ] P3.9 Same-origin / CSRF gate on every API route
- [ ] P3.10 Strict CSP; the terminal is the most attacker-influenced surface in the product
- [ ] P3.11 Bind policy: loopback by default, explicit opt-in, loud about which is active
- [ ] P3.11b `trust_proxy` setting governing `X-Forwarded-*` — opt-in, never inferred
- [ ] P3.11c Own-TLS mode (cert/key files) as an alternative to a reverse proxy
- [ ] P3.12 Security review of the whole surface before the first public bind

**Gate:** P3.12 signed off. Nothing binds off-loopback before this.

---

## Phase 4 — Multi-remote mesh (4–5 days)

Herdr contributes nothing here and blocks nothing (findings §1.8, §3.1).

- [x] P4.1 Node identity: keypair per node, generated on first run
- [x] P4.2 Node-to-node transport: outbound-dialling peers → hub, so NAT'd hosts join without inbound ports
- [x] P4.3 Mesh auth **separate from user auth** (mTLS or per-node token); a compromised viewer can't impersonate a node
- [ ] P4.4 Node enumerates **all named sessions** on its host (`herdr session list --json`), not just default
- [x] P4.5 Merged herd model: stable global ids `node/session/workspace/tab/pane`
- [x] P4.6 Frame relay hub→peer with the Phase 1 backpressure rules applied per hop
- [x] P4.7 Per-node health, version skew display, and graceful degradation when a node drops
- [x] P4.8 Any node can be hub — role is config, not a separate build
- [~] P4.9 Per-node latency indicator in the UI (a 200 ms peer should look different from a local pane)

- [x] P4.10 Enrolment and revocation: single-use join codes, a pinned hub key, a visible peer list
- [x] P4.11 The deployment written down — one hub behind Nginx Proxy Manager (`docs/07-mesh-deployment.md`)

**Gate:** two hosts, one hub, panes from both drivable from one phone. **Met on one machine**
(`crates/kampr-node/tests/mesh.rs`: two nodes, two herdr sessions, one hub — a peer pane renders and
takes input through the hub, the peer dying degrades only its own panes, and it recovers unaided).
P4.9 is the client half of the latency indicator: the node now ships a measured `rtt_ms` and a
per-node `build`, and the UI has still to render them.

---

## Phase 4.5 — Herd management: parity with the TUI (3–4 days)

Everything you would do at the keyboard. Probes #46–#50 confirm the socket carries all of it except
named-session creation, which shells out. Depends on Phase 4's node model for the `node` field.

- [x] P4.5.1 `manage` op dispatch on the node, with `hello.caps.manage` and per-op `not_writer` gating
- [x] P4.5.2 Structure: `workspace.create` / `tab.create` / `pane.split` / `pane.zoom` / rename / close / focus
- [x] P4.5.3 `env` and `cwd` on create — "new session in this worktree with these variables" is one call
- [x] P4.5.4 `agent.start`, with kinds from `server.agent_manifests` at runtime, **never a hardcoded client list**
- [~] P4.5.5 Worktrees: list / create / open / remove — Herdr's git support maps straight through
- [~] P4.5.6 Layouts: `layout.export` → store → `layout.apply`; named layouts a user can re-apply
- [x] P4.5.7 Named sessions: enumerate via `herdr session list --json`, create via a headless `herdr server --session`, stop and delete. The one management path that shells out
- [ ] P4.5.8 **Kampr split view** — a client-side mosaic of 2–4 panes that may come from different sessions on different nodes. The TUI cannot do this; it is the clearest place Kampr beats it
- [ ] P4.5.9 Split view on mobile: one pane at a time with a fast switcher, not a squeezed mosaic
- [x] P4.5.10 Structural actions state their effect before acting — a split reshapes the pane for everyone
- [x] P4.5.11 Clients never optimistically mutate the herd model; they wait for the `herd.patch`
- [ ] P4.5.12 `notification.show` — raise a desktop toast from the phone

**Gate:** create a workspace, split it, start a Claude agent in the new pane, and watch two panes from
two different nodes side by side — all from a phone. **The management half is met**
(`crates/kampr-node/tests/live.rs::every_client_op_lands_on_a_real_herd` drives every op the client
can build against a real herd, and `client/shared`'s `LiveNodeTest` drives the same ops through the
real client and waits for the `herd.patch`). P4.5.5 and P4.5.6 have the ops and the wire but no
list/remove and no stored named layouts; P4.5.8/P4.5.9 — the client-side mosaic — are still the
missing half of the gate.

---

## Phase 5 — Conversation view: real markdown (4–5 days)

Also the **default view for agent panes** (findings §3.8), which is what keeps a 94-column grid off a
phone screen in the common case.

The answer to "unlike Collie which just wraps text". Structure cannot come from the terminal stream
(findings §3.6) — it comes from the transcripts.

- [ ] P5.1 Journal adapter interface + registry keyed on Herdr's `agent` string (Collie's `bridge/journal/` shape)
- [ ] P5.2 Claude adapter — `~/.claude/projects/<slug>/<uuid>.jsonl`
- [ ] P5.3 Codex adapter — `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`
- [ ] P5.4 Path containment: transcript roots are containment roots, never request input
- [ ] P5.5 Tail-follow with incremental parse; new turns push over the existing WS
- [ ] P5.6 Markdown renderer: **tables as real tables** (horizontally scrollable on phones), fenced code with syntax highlighting + copy, nested lists, blockquotes
- [ ] P5.7 Tool calls / results collapsible; diffs rendered as diffs
- [ ] P5.8 Token/cost line where the harness records it
- [ ] P5.9 View toggle: Terminal ⇄ Conversation, per pane, state remembered per device
- [ ] P5.9b **Default-view rule** from the pane's `agent` field: agent + adapter → Conversation; agent without adapter → Terminal; no agent → Terminal with the Conversation tab hidden
- [ ] P5.9c **Answer a blocked prompt from Conversation** — question from the transcript, answer as `send_keys`. No lease, no terminal stream
- [ ] P5.10 Graceful "no adapter for this harness" — terminal view only, stated plainly
- [ ] P5.11 Search across the whole transcript (not just the viewport — the thing the mirror can never do)

**Gate:** a Claude session with a markdown table renders as a table on a phone, and the same pane still drives live in Terminal view.

---

## Phase 6 — Responsive UI, properly (4–5 days)

- [ ] P6.1 Desktop: full-viewport terminal, herd navigator, no wasted chrome
- [ ] P6.2 Desktop multi-pane layout for the split view built in P4.5.8 — sizing, focus, drag to rearrange
- [ ] P6.3 Mobile portrait: full-bleed terminal, key row docked to keyboard, nav as a sheet
- [ ] P6.4 Mobile landscape: maximum terminal, collapsible key row, nav as an overlay
- [ ] P6.5 **Readability without resizing** — landscape fits natively; Conversation is the agent default; portrait terminal is zoom and pan
- [ ] P6.5b Landscape is a first-class layout, not a rotation fallback — 844 px fits the native 94 columns at 13 px with no zoom at all
- [ ] P6.6 Font size / zoom control with column count shown live
- [ ] P6.7 Safe-area insets, notches, `visualViewport`, and no body scroll — ever
- [ ] P6.7b **Terminal is full-bleed on every breakpoint** — chrome floats over it, never insets it
- [ ] P6.7c **Scrollback and the live grid are one continuous surface**; blank space below the last row is a bug
- [ ] P6.7d Default zoom fills at least one axis (`max(fit-width, fit-height)`), never letterboxes
- [ ] P6.7e Make the node's ring bound configurable and generous; no client-side row cap
- [ ] P6.8 Ship the four themes as a user setting (soft default); confirm every screen holds at each
- [ ] P6.8b Light-ground variant, honouring `prefers-color-scheme` — note Herdr answers no OSC 10/11 background query, so harness output is authored dark (Collie ADR 0002), which a light theme has to reckon with
- [ ] P6.8c ANSI palette mapping per theme — the terminal's 16 slots should agree with the chrome
- [ ] P6.9 Agent-status triage list — the one Collie product idea worth stealing wholesale (NEEDS YOU first)
- [ ] P6.10 PWA: manifest, service worker, build stamp cache-bust
- [ ] P6.11 Reduced-motion and screen-reader pass on navigation (the terminal itself is a known hard case)

**Gate:** the design canvas (`docs/design/`) and the shipped UI agree at all three breakpoints.

---

## Phase 7 — Setup ladder & lifecycle (2–3 days)

- [ ] P7.1 `herdr-plugin.toml`: `[[actions]]` (start/stop/restart/status/url/update/uninstall), `[[panes]]` popup setup, `[[startup]]` nudge, `min_herdr_version` from P0.5
- [x] P7.1b `[[build]]` downloads a prebuilt binary — `packaging/fetch-binary.sh`, os/arch matched
- [x] P7.1c Standalone route written — `packaging/install.sh`
- [ ] P7.1d Single-binary packaging: Gradle builds the CMP wasm bundle, `rust-embed` bakes it in
- [ ] P7.2 Terminal wizard as `placement = "popup"`, `width = "80%"` — first-run setup, session-modal
- [ ] P7.3 The ladder screen: running-now card (URL, QR, pairing code) plus optional upgrades, each labelled with what it unlocks
- [ ] P7.3b Copy-paste reverse-proxy snippets for NPM, Caddy and Traefik — NPM with a DNS-01 wildcard is the documented default, since it gives a real cert on a LAN-only hostname with nothing exposed
- [x] P7.4 Supervision: `packaging/kampr.service` template + `kamprctl.sh` installs and nudges it; the launchd branch renders `packaging/dev.kampr.node.plist` and bootstraps it, and only reloads when the plist actually changed — never run on a Mac
- [x] P7.5 `update` action owns the refresh, since plugin v1 has none
- [ ] P7.6 Browser first-run wizard mirroring the terminal one, for people who start from the phone
- [x] P7.7 `kampr doctor` — herdr socket and version floor, sessions, bind and tier, TLS or proxy, file modes, client bundle, service state, devices and recovery; `--json`, non-zero exit on a real failure. Peer health is not covered while the mesh is in flight
- [x] P7.8 Uninstall that actually cleans up: `uninstall` removes service and units and says where the devices still live; `purge` removes those too. `purge` is deliberately not a Herdr action — an action list is one tap away from a phone
- [x] P7.9 Release workflow: Gradle stages the bundle, `build.rs` refuses to build a binary without it, `cross` builds static musl for linux x86_64/aarch64 and macOS for both arches, `SHA256SUMS` is signed keyless with cosign, and a clean runner installs the artefact and runs it — unexercised until the first tag

**Gate:** a clean machine goes from `herdr plugin install` to a working authenticated phone session with no manual file editing.

---

## Phase 8 — Notifications & polish (2–3 days)

- [x] P8.1 Subscribe `pane.agent_status_changed` per agent pane; resubscribe when the pane set changes — debounced at 500 ms so a workspace of ten agents is one resubscribe; events poke the poll rather than replacing it
- [x] P8.2 Web Push (VAPID) for blocked agents on Tier 1+; batch simultaneous blocks into one notification — 900 ms window, split per subscription so a mute removes one agent rather than the batch
- [x] P8.2b iOS: Add-to-Home-Screen prompt, since Web Push works nowhere else on iOS — detected, prompted, and the `apple-mobile-web-app-*` tags that make the installed app notifiable. **Never run on an iPhone.**
- [x] P8.2c UnifiedPush as an alternative Android transport, for self-hosters avoiding FCM — **decided and server-complete**: a distributor endpoint is an RFC 8291 endpoint, so the sender is unchanged. The client half needs a distributor the user installs and is documented, not assumed
- [x] P8.2d Service worker **prefetches on push** — `/api/node` + `/api/warm?pane=`, observed live in the browser's cache within the push handler. Not the grid: that would be a second encoder, and the socket delivers the real one within a second
- [x] P8.3 **Put the question in the notification body** — delivered live as `body="Do you want to proceed?"`. Sourced from the screen, not the transcript (probe #42)
- [x] P8.4 Deep link: notification → that pane, correct view — one pane opens Conversation, a batch opens the triage list. The payload carries it; the click itself is untested
- [x] P8.5 Per-agent snooze / mute — per device, wildcard for the whole herd, snooze expires in the query so nothing sweeps
- [ ] P8.6 Connection state UI for both loops (browser↔node, node↔herdr, node↔peer)
- [ ] P8.7 Offline behaviour: last frame frozen and clearly marked stale, not silently wrong

---

## Phase 8.5 — Kampr on Android *(optional, gated on the provider seam)*

Only worth starting once P1.3b exists. iOS has no equivalent and will not get one — third-party apps
cannot spawn processes (findings §3.10).

- [ ] P8.5.1 Local-PTY provider: fork/exec + PTY inside the app sandbox, the way Termux does it
- [ ] P8.5.2 Android node packaging — the phone joins the mesh as a peer
- [ ] P8.5.3 Be honest about scope: app-uid shell, app sandbox and shared storage, not root
- [ ] P8.5.4 Decide native app vs PWA once push and background behaviour are measured in the field

---

## Phase 9 — Release

- [ ] P9.1 README built around the tier ladder — IP+port first, then hostname+cert (NPM/Caddy/Traefik), then public or Tailscale as equals
- [ ] P9.2 ARCHITECTURE.md — the reasoning, in Collie's style, since it's the reason Collie is maintainable
- [ ] P9.3 ADRs for the four inversions: stream-not-poll, emulator-in-browser, auth-in-bridge, mesh
- [ ] P9.4 `docs/03-probe-log.md` kept current — every claim about Herdr traceable to a command
- [x] P9.5 CI: fmt, clippy (`-D warnings`), tests, Gradle build, shell syntax, manifest validation
- [ ] P9.6 GitHub topic `herdr-plugin` for marketplace discovery
- [ ] P9.7 Publish

---

## Cut / not doing

- [-] Reverse-engineering `herdr-client.sock` (bincode, private, breaks every release — findings §1)
- [-] Recovering markdown structure by re-parsing the terminal grid (information is gone; Phase 5 is the real answer)
- [-] A second terminal emulator in the bridge (Herdr already rendered; two renderers disagree — Collie ADR 0008 is right about this)
- [-] Forking Collie (different product; the collision is architectural, not incidental)
- [-] Trusted-header auth as the *only* gate (it stays available behind an opt-in `trust_proxy`, but never as the credential)
- [-] Reflowing terminal rows into wrapped text for phone width (this is Collie's answer; the ladder in P6.5 replaces it)
- [-] Resizing a pane for any reason. Kampr uses `observe` + JSON-RPC input and cannot reshape a session.
- [-] `terminal session control` in any form — it always claims geometry, with no flag to decline.
- [-] A VT emulator in Kotlin. It runs once, in Rust, on the server.
- [-] Requiring Tailscale for anything. It is one way up the ladder, alongside NPM / Caddy / Traefik.
- [-] A local shell on iOS. Third-party apps cannot spawn processes; no amount of effort changes it.
- [-] A six-step first-run wizard. Setup starts from working and offers labelled upgrades.

## Upstream asks for Herdr

- [ ] U1 A subscribable output/revision-changed event, so the JSON API alone can drive a live UI
- [ ] U2 Reflow (not crop) for `observe --cols/--rows`, which would remove the geometry trade in §3.4 entirely
- [ ] U3 Per-client geometry that doesn't resize the shared PTY (the general form of U2)
- [ ] U4 OSC 8 hyperlinks preserved through `pane.read`
- [ ] U5 `Home`/`End`/`PageUp`/`PageDown`/`Insert`/`Delete` in the `send_keys` grammar
- [ ] U6 A protocol version for the terminal stream contract, independent of the socket protocol
- [ ] U7 Expose attached-client count / geometry in `session.snapshot`, so a bridge can say whether anyone is at the desk
- [ ] U8b An event on native geometry change — today a bridge must poll to notice the desk resizing (#52)
- [ ] U8c An offset parameter on `pane.read recent`, so deep scrollback is reachable at all (#51)
- [ ] U8 A read-only *input* mode for `terminal session control`, or `terminal.scroll` on an observer — so a bridge can scroll without claiming the PTY
- [ ] U9 `observe` following native geometry automatically, instead of defaulting to 120×40
