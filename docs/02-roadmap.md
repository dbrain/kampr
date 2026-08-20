# Kampr — roadmap

Tick as you go. Derived from `01-implementation-findings.md`; every item traces to a finding there.

**Legend:** `[ ]` todo · `[x]` done · `[~]` in progress · `[!]` blocked · `[-]` cut
**Gate** = do not start the next phase until this is true.

---

## Phase 0 — Close the probe gaps (½ day)

The seven open questions in findings §6. Cheap, and each one can invalidate a Phase 2–4 decision.

- [x] P0.1 **OSC 8 survives `terminal.frame`** — the spike interned a link from the frame stream. `pane.read` drops it, so frames are strictly richer
- [ ] P0.2 `terminal.closed` reason + latency when a streamed pane exits → decides reconnect UX
- [ ] P0.3 Does `observe`/`control` accept an agent-name target as well as `wN:pN`?
- [x] P0.4 **Scrollback answered without `terminal.scroll`** — frames carry end state only; the ring comes from `pane.read recent` (instant, colour-preserving, viewport unmoved) and alt-screen panes have no ring at all
- [ ] P0.5 Pin the version floor for `observe`/`control` + `--cols/--rows` → sets `min_herdr_version`
- [ ] P0.6 Frame-stream cost: CPU/RSS of N concurrent `observe` children on one host → sizes the mux
- [ ] P0.7 Behaviour when the *same* pane is observed at two different geometries simultaneously (already seen working; confirm no cross-talk under load)
- [x] P0.9 **Is a control-mode resize reversible?** Yes — but moot: `control` always claims geometry, so Kampr does not use it
- [x] P0.10 **Can Kampr tell whether a desktop client is attached?** No — and now moot, since nothing is ever borrowed
- [x] P0.11 **Can input be sent without claiming geometry?** Yes — `pane.send_text` / `pane.send_keys` over JSON-RPC. `control` cannot: no-flag control still takes the PTY to 120×40
- [x] P0.13 **`observe` at native size is pixel-exact** — `kampr-spike` matches `pane.read visible` 30/30 rows, cursor included
- [x] P0.14 **`pane.read recent` on a shell pane is safe** — 0.002 s, viewport unmoved, 256-colour intact
- [ ] P0.15 **Is a pending tool request in the transcript before approval?** Decides the answer strip's source
- [ ] P0.16 **CMP wasm spike**: 94×40 cell grid at 60 fps on a mid-range Android — before the client architecture is committed
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
- [ ] P1.2b Theme token layer first, not last: every colour, font, radius and border through a CSS custom property, `data-theme` on the root, `soft` shipping and `phosphor` / `warm` / `brutalist` defined alongside it (see `docs/design/build.py`)
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
- [ ] P2.2b Zoom presets (fit width / readable / close up) and per-pane per-device persistence
- [ ] P2.3 Keystroke coalescing per animation frame, ordered, with an in-flight cap
- [ ] P2.5 Raw-escape key table — Esc, Tab, arrows, Home, End, PgUp, PgDn, Del, Ins, F1–F12 — as **bytes**, never as `send_keys` names (findings §1.4)
- [ ] P2.6 JuiceSSH-style key row: two rows above the native keyboard, latching Ctrl / Alt / Shift / Fn modifiers
- [ ] P2.7 Long-press on a key row button → alternates (e.g. Ctrl long-press → Ctrl-C/D/Z/L/R shortcuts)
- [ ] P2.8 Native soft keyboard for text so dictation keeps working; visualViewport handling so the row sits on the keyboard, not under it
- [ ] P2.9 Paste: bracketed paste framing done by us, since `pane.send_text` writes raw bytes with no framing
- [ ] P2.10 Destructive-command confirm (lift Collie's `destructive.ts` pattern) — but on the *composed line*, not per keystroke
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
- [ ] P3.5 Recovery code, generated once, shown once
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

- [ ] P4.1 Node identity: keypair per node, generated on first run
- [ ] P4.2 Node-to-node transport: outbound-dialling peers → hub, so NAT'd hosts join without inbound ports
- [ ] P4.3 Mesh auth **separate from user auth** (mTLS or per-node token); a compromised viewer can't impersonate a node
- [ ] P4.4 Node enumerates **all named sessions** on its host (`herdr session list --json`), not just default
- [ ] P4.5 Merged herd model: stable global ids `node/session/workspace/tab/pane`
- [ ] P4.6 Frame relay hub→peer with the Phase 1 backpressure rules applied per hop
- [ ] P4.7 Per-node health, version skew display, and graceful degradation when a node drops
- [ ] P4.8 Any node can be hub — role is config, not a separate build
- [ ] P4.9 Per-node latency indicator in the UI (a 200 ms peer should look different from a local pane)

**Gate:** two hosts, one hub, panes from both drivable from one phone.

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
- [ ] P6.2 Desktop multi-pane: split view of 2–4 panes (each its own stream at its own geometry — findings §1.6 says this is free)
- [ ] P6.3 Mobile portrait: full-bleed terminal, key row docked to keyboard, nav as a sheet
- [ ] P6.4 Mobile landscape: maximum terminal, collapsible key row, nav as an overlay
- [ ] P6.5 **Readability without resizing** — landscape fits natively; Conversation is the agent default; portrait terminal is zoom and pan
- [ ] P6.5b Landscape is a first-class layout, not a rotation fallback — 844 px fits the native 94 columns at 13 px with no zoom at all
- [ ] P6.6 Font size / zoom control with column count shown live
- [ ] P6.7 Safe-area insets, notches, `visualViewport`, and no body scroll — ever
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
- [ ] P7.1b `[[build]]` **downloads a prebuilt binary** rather than compiling — no Rust toolchain on the user's machine
- [ ] P7.1c Standalone route: `install.sh` → `kampr init` → `kampr service install`, with no Herdr plugin involved
- [ ] P7.1d Single-binary packaging: Gradle builds the CMP wasm bundle, `rust-embed` bakes it in
- [ ] P7.2 Terminal wizard as `placement = "popup"`, `width = "80%"` — first-run setup, session-modal
- [ ] P7.3 The ladder screen: running-now card (URL, QR, pairing code) plus optional upgrades, each labelled with what it unlocks
- [ ] P7.3b Copy-paste reverse-proxy snippets for NPM, Caddy and Traefik — NPM with a DNS-01 wildcard is the documented default, since it gives a real cert on a LAN-only hostname with nothing exposed
- [ ] P7.4 Supervision: generate + install systemd `--user` unit (launchd on macOS); `[[startup]]` hook re-nudges after Herdr restart and live handoff
- [ ] P7.5 Self-update action — plugin v1 has no `plugin update`, and reinstall restarts nothing (findings §1.9)
- [ ] P7.6 Browser first-run wizard mirroring the terminal one, for people who start from the phone
- [ ] P7.7 `kampr doctor` — checks socket reachability, Herdr version floor, port bind, TLS cert, peer health, and prints what's wrong in one screen
- [ ] P7.8 Uninstall that actually cleans up: service, units, tokens, state

**Gate:** a clean machine goes from `herdr plugin install` to a working authenticated phone session with no manual file editing.

---

## Phase 8 — Notifications & polish (2–3 days)

- [ ] P8.1 Subscribe `pane.agent_status_changed` per agent pane; resubscribe when the pane set changes
- [ ] P8.2 Web Push (VAPID) for blocked agents on Tier 1+; batch simultaneous blocks into one notification
- [ ] P8.2b iOS: Add-to-Home-Screen prompt, since Web Push works nowhere else on iOS
- [ ] P8.2c UnifiedPush as an alternative Android transport, for self-hosters avoiding FCM
- [ ] P8.2d Service worker **prefetches on push** — snapshot + that pane's full frame into cache, so the tap opens onto warm data
- [ ] P8.3 **Put the question in the notification body** — Collie's documented known gap, and we have the transcript to source it from
- [ ] P8.4 Deep link: notification → that pane, correct view
- [ ] P8.5 Per-agent snooze / mute
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
- [ ] P9.5 CI: typecheck, tests, plugin manifest validation
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
- [ ] U8 A read-only *input* mode for `terminal session control`, or `terminal.scroll` on an observer — so a bridge can scroll without claiming the PTY
- [ ] U9 `observe` following native geometry automatically, instead of defaulting to 120×40
