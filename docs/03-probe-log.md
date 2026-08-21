# Probe log

> **Probe numbers are permanent identifiers, not positions.** Code and docs cite them, so renumbering
> silently repoints a citation at unrelated evidence — which has happened once already: four sites
> citing #75 for the PTY/rect divergence ended up pointing at an `agent_session` finding after a
> renumber. Append new rows; never renumber an existing one. If a probe is superseded, strike it
> through and reference the row that replaced it.
>
> **The rule was broken three times by parallel work, and repaired on 2026-08-21.** Numbers 77, 78
> and 92 each ended up on two different rows, because agents working at the same time each appended
> at what was then the end. A duplicate cannot be left alone — it makes every citation ambiguous —
> so in each pair the row with the *fewest* citations moved: #77→#105 and #78→#106 (both uncited),
> and #92→#107, whose one citation in `kampr-core/src/herdr_provider.rs` moved with it. The
> accessibility #92 kept its number so the `#92–#94` range in ADR 0010 stayed intact. If you are
> appending from a parallel task, propose the row and let one writer assign the number.


Every claim Kampr makes about Herdr, traced to the command that produced it. All against
**herdr 0.8.2, protocol 20** on 2026-08-20, in throwaway named sessions (`kp2`…`kpspike`), all torn
down afterwards. Tooling: `research/probe/rpc.py`, `research/probe/ptyclient.py`.

Add to this file rather than re-deriving. A claim without a row here is a guess.

## Transport and API

| # | Claim | How | Result |
|---|---|---|---|
| 1 | Socket is NDJSON, one request per connection | write + read one line | Confirmed; `events.subscribe` is the only stream |
| 2 | 91 methods, protocol 20 | `herdr api schema --json` | `research/herdr-methods.md` |
| 3 | `revision` is live again on 0.8.2 | `pane.list` | Non-zero, unlike the 0.7.x stub Collie recorded |

## Events

| # | Claim | How | Result |
|---|---|---|---|
| 4 | 27 subscribable types; **no output event** | subscribe to `bogus.type`, read the validator's list | `pane.output_changed` is an EventKind but **not subscribable** |
| 5 | `pane.updated` does not fire on output | subscribe, run a 3 s output loop | 0 events |
| 6 | `pane.wait_for_output` is a match-waiter, not a change-waiter | regex `.` | Returns in 0.00 s against existing content; ~110 ms for new content |

## Keys and input

| # | Claim | How | Result |
|---|---|---|---|
| 7 | Key grammar | probe 46 names against a live pane | `Up Down Left Right Tab Enter Escape Space Backspace BS F1–F13`, single chars, `ctrl+`/`alt+`/`shift+` chords |
| 8 | `PageUp PageDown Home End Insert Delete BackTab` rejected | same | `invalid_key` on 0.8.2 — still |
| 9 | `pane.send_text` writes **raw bytes** | send into `cat -v` | `^[`, `^[[5~`, `^[[H`, `^A`, UTF-8 all arrive intact — so any key is sendable |

## Terminal streams

| # | Claim | How | Result |
|---|---|---|---|
| 10 | Frame shape | run `observe` | `{type:"terminal.frame", seq, width, height, encoding:"ansi", full, bytes:<b64>}`, then `terminal.closed{reason}` |
| 11 | Control stdin grammar | send `{"type":"bogus"}` | `terminal.input` (`text` or `bytes`), `terminal.resize` (`cols`,`rows`), `terminal.scroll` (`direction` up/down, `lines` u16), `terminal.release` |
| 12 | Frames carry cursor + sync | decode | `ESC[r;cH`, `ESC[?25h/l`, wrapped in `ESC[?2026h…l` |
| 13 | Many observers coexist, each at its own size | two observers at 80×24 and 40×12 | Both streamed; PTY untouched |
| 14 | `observe` never touches the PTY | observer at 60×20 on a 36-row pane | PTY stayed 36 |
| 15 | `observe --cols` **crops, never reflows** | 120-char line, 93-col grid, observed at 60 | Columns 61–93 lost, not wrapped |
| 16 | `observe` with no size flags defaults to **120×40** | omit the flags | Not native size — always pass the real geometry |
| 17 | `control` **always** claims the PTY | control with no size flags | PTY forced to 120×40; there is no opt-out |
| 18 | A controller overrides the desktop while held | resize desktop to 120×44 during control | Ignored; PTY stayed at the controller's size |
| 19 | Control resize is released automatically | `terminal.release`, then `SIGKILL` | Desktop geometry restored, ≤1 s in both cases |
| 20 | A *frozen* controller holds it forever | `SIGSTOP` | Held; Herdr never reclaims |
| 21 | One controller at a time | second controller | `terminal.closed: already has an attached client; retry with --takeover`; `--takeover` evicts |
| 22 | Echo latency | 10 keystrokes through control | **p50 27 ms, p90 98 ms, min 17 ms** |
| 23 | Heavy output coalesces | `seq 1 20000` under observe | **3 frames, 1.9 KB total** — grid state, not a byte replay |
| 24 | Works fully headless | `herdr server --session X`, create workspace over the API, observe | Streams with no TUI client ever attached |

## Scrollback

| # | Claim | How | Result |
|---|---|---|---|
| 25 | **Frames carry END STATE ONLY** | `seq 1 200` on a 30-row pane, union of all frames | Only 29 distinct lines (172–200). Lines 1–171 never transmitted — a frame-fed emulator **cannot** rebuild scrollback |
| 26 | Herdr holds the ring | `pane.get` after | `max_offset_from_bottom: 171` |
| 27 | `pane.read recent` on a **shell** pane is instant and safe | `lines=400` | **0.002 s**, 401 lines, all 200 markers, viewport **unmoved** |
| 28 | ANSI scrollback keeps colour | `recent format=ansi`, 400 × 256-colour lines | 0.001 s, 11.7 KB, 400/400 markers, 1200 SGR runs, viewport unmoved |
| 29 | ~~Over-asking clamps harmlessly~~ **CORRECTED by #51** | `lines=5000` against a 400-row ring | Returned 400, `truncated:false`. That only held because the ring was shallower than herdr's cap — see #51 |
| 30 | **Alt screen has no ring** | `ESC[?1049h`, then read | `max_offset_from_bottom: 0`; `recent` degrades to the viewport, instantly, unmoved. Exiting alt screen restores the ring |

> **Interlock.** Read scrollback only when `max_offset_from_bottom > 0` **and** the pane has no
> detected `agent`. The second half is Collie's documented hazard: on an idle *recognised agent* pane,
> `recent` with `lines > viewport_rows` harvests via the agent's own mouse-scroll interface — slow,
> and it moves the operator's screen. Encoded as `Pane::scrollback_is_safe_to_read`.

## Geometry

| # | Claim | How | Result |
|---|---|---|---|
| 31 | PTY size comes from the attached client; last writer wins | clients at 100×30 and 60×20 | Became 18 rows; resizing the first to 200×50 took it back, with the small client still attached |
| 32 | `pane.resize` is layout-only | single-pane tab | `{changed:false, reason:"unchanged"}` |
| 33 | Native geometry is in the layout rect | `session.snapshot` → `layouts[].panes[].rect` | Confirmed: 100-col client → 74-col pane rect (sidebar takes 26) |
| 34 | Nothing reports attached clients | `ping`, `session.snapshot`, `herdr status` | Absent everywhere |

## Reads and agents

| # | Claim | How | Result |
|---|---|---|---|
| 35 | `format:"ansi"` keeps bold/italic/underline/reverse/blink/256/truecolour | printf probe | All preserved |
| 36 | `pane.read` **drops OSC 8 hyperlinks** | `ESC]8;;url…` | Round-trips as bare text |
| 37 | **Frames keep OSC 8** | spike interned links from the frame stream | 1 hyperlink recovered — the frame path is strictly richer than `pane.read` |
| 38 | Shell panes carry no `agent` key | `pane.list` | Agent-vs-shell discriminator confirmed |
| 39 | Claude transcripts hold raw markdown | parse `~/.claude/projects/**/*.jsonl` | 676 assistant records, literal markdown |
| 40 | Every `tool_use` in a finished session has a `tool_result` | same | 300/300 — proves nothing about whether a *pending* request is flushed before approval. **Resolved by #42/#43** |

## End-to-end

| # | Claim | How | Result |
|---|---|---|---|
| 41 | **`observe` → emulator → cell grid reproduces herdr's own grid exactly** | `cargo run -p kampr-spike`, compared against `pane.read visible` | **30/30 rows identical**, cursor at the right cell, 1 hyperlink interned. Truecolour, 256-colour, reverse, underline, box drawing and non-ASCII all round-trip |

Reproduce #41:

```bash
herdr --session probe                        # in one terminal
HERDR_SESSION=probe cargo run -p kampr-spike # in another
```

## Transcripts

| # | Claim | How | Result |
|---|---|---|---|
| 42 | **Claude does NOT write a pending tool request to the transcript before approval** | `claude --permission-mode default` in a scratch dir under a pty, prompted to run `touch marker.txt`, then held at the permission prompt while polling `~/.claude/projects/<slug>/<uuid>.jsonl` | The transcript froze at **15 680 bytes with the user record only** for **4 m 20 s** of prompt time — zero `tool_use` blocks. 19 s after answering `1` it jumped to 20 469 bytes carrying **both** the `tool_use` and its `tool_result`. Repeated with an automated driver: same result. Claude Code 2.1.237 |
| 43 | **Codex DOES write a pending tool request to the rollout before approval** | `codex -a untrusted -s read-only` under a pty, held 54 s at "Press enter to confirm", polling the newest `~/.codex/sessions/**/rollout-*.jsonl` each second | `custom_tool_call` present **6.2 s in, with the prompt still on screen** and `custom_tool_call_output` absent. The output only appeared 1 s after the keypress, and reported `Wall time 54.8 seconds`. **An unmatched tool call is therefore Codex's pending signal.** codex-cli 0.147.0 |
| 44 | `~/.claude/sessions/<pid>.json` flags the block but not the question | read it while #42 was held | `"status":"waiting","waitingFor":"permission prompt"` — a cheap *detector*, but it carries no question text and no options, so the wording still has to come from the screen |
| 45 | Codex rollout schema | parsed 5 real rollouts, cli 0.131.0 → 0.147.0, 13 k records | Envelope `{timestamp, type, ordinal?, payload}`. `type` is `session_meta` \| `turn_context` \| `world_state` \| `response_item` \| `event_msg` \| `compacted`. **Only `response_item` carries the conversation**; `event_msg` (`agent_message`, `item_completed`) duplicates it for the TUI, one-for-one. Payload types: `message` (role `user`\|`assistant`\|`developer`, `content[].type` `input_text`\|`output_text`, assistant carries `phase` `commentary`\|`final_answer`), `function_call`/`function_call_output` (`exec_command`, `write_stdin`, `update_plan`, `view_image`; `arguments` is a JSON **string**), `custom_tool_call`/`custom_tool_call_output` (`apply_patch`, and `exec` in 0.147 code mode carrying JavaScript), `web_search_call`, `reasoning`. **`reasoning` is always `encrypted_content` with an empty summary — there is no plaintext thinking to render.** Outputs are a string in ≤0.131 and an array of content items in 0.147. `compacted.replacement_history` rewrites model context, not display history |

> **Consequence for `pending`.** Claude is the harness Kampr targets first and it does not publish
> the question until it has already been answered, so the node sources `pending` from
> `pane.read visible` and sets `source: "screen"` (#42). Codex could be read from the transcript
> (#43) — an unmatched `custom_tool_call` — but the wire shape is identical either way, so the
> screen path is the one implementation and `source` is the only thing that differs.

## Corrections and event behaviour

| # | Claim | How | Result |
|---|---|---|---|
| 51 | **`pane.read recent` caps at 1000 lines, and deeper history is unreachable** | 1400-line ring (`max_offset_from_bottom: 1371`), asked for 400 / 1200 / 5000 | 400 → 400 rows, 1200 → **1000**, 5000 → **1000**, all `truncated: true`. `pane.read` has no offset parameter, so there is **no way to page further back**. Corrects #29 |
| 52 | **No event fires when the attached client resizes** | subscribed `layout.updated`, `pane.updated`, `pane.moved`, `workspace.updated`, `tab.focused`, `pane.focused`; resized the desk client three times, verifying the pane rect moved 74 → 94 → 54 → 114 columns each time | **Zero events, all six types, all three resizes.** Control: a `pane.split` fired `layout_updated` *and* `pane_updated`. So `layout.updated` covers structural change only — **native geometry change from the desk is detectable only by polling** |
| 53 | Only the first frame of an `observe` stream is `full: true` | 19 frames over 12 s of shell activity | 1 full. Mapping `full` → `grid.reset` costs nothing |
| 54 | A subscription list is all-or-nothing | include `pane.scroll_changed` without a `pane_id` | One invalid entry rejects the whole `events.subscribe` call, not just that entry |
| 55 | Herdr's `truncated` means "there was more than you asked for" | #51 | Not "we hit the cap" — a short read can also be truncated |
| 107 | **A subscription list is all-or-nothing twice over, and a stale `pane_id` is the second way** | subscribed `pane.agent_status_changed` with `pane_id: w1:p1` (accepted), then the same list plus `pane_id: w9:p9` | The ghost entry answers `{"id":"…:sub:1:…","error":{"code":"pane_not_found"}}` **as the ack**, and the socket closes — so a pane that shut between the snapshot and the subscribe kills the whole call exactly as a missing `pane_id` does (#54), just at a later stage. **An already-open subscription survives its pane closing**: closing `w1:p2` under a live subscription delivered `pane_closed` and left the stream up. So only the initial call is exposed, and the fix is to re-derive from a fresh snapshot and retry |
| 77 | **`notification.show` reports whether anyone saw it** | called it against a headless session, then with a `herdr --session kpush` client attached under a pty | Headless: `{shown:false, reason:"no_foreground_client"}`. With a client: `{shown:true, reason:"shown"}`. `level` is not a parameter and `body` is optional. A headless session is what the plugin and the systemd unit both produce, so a node must relay `shown` rather than reporting success. The rendered text was **not** observed in the client's PTY byte stream, so the toast's appearance is unverified — only herdr's own claim about it |
| 78 | **`pane.agent_status_changed` beats the 3 s poll by ~2.3 s** | `agent.start` a real `claude` in a throwaway session, subscribe per pane, drive it to a Bash permission prompt while a 3 s `session.snapshot` poll runs alongside; five runs | Event first every time, by **1.38 / 2.21 / 2.58 / 2.67 / 2.84 s** — mean **2.33 s**. Payload is `{agent, agent_status, pane_id, workspace_id}` under `event: "pane_agent_status_changed"`. That interval is what the whole triage story used to spend waiting |


## Herd management (feature parity with the TUI)

| # | Claim | How | Result |
|---|---|---|---|
| 46 | Structure is fully writable over the socket | `workspace.create`, `tab.create`, `pane.split` right + down, `pane.zoom` on/off, `workspace.rename`, `workspace.close` against a throwaway session | **All succeeded.** Ended at 2 workspaces / 3 tabs / 5 panes, created entirely over the API with no keystrokes |
| 47 | `layout.export` returns a nestable split tree | export a workspace with two splits | `{root:{type:"split", direction:"right", ratio:0.5, first:{type:"split", direction:"down", …}}}` — and `layout.apply` takes the same shape, so layouts are savable and restorable |
| 48 | `agent.start` launches a harness into a pane | bogus kind first, then `server.agent_manifests` | Bogus → `unsupported_agent_kind`. The valid set is discoverable at runtime: **20 kinds on this host** (`claude codex gemini grok copilot cursor devin droid amp cline kilo kimi kiro maki hermes opencode pi qwen qodercli agy`) |
| 49 | Named sessions are enumerable and creatable without a TTY | `herdr session list --json`; `herdr server --session <name>` | Listing gives `{name, running, session_dir, socket_path}` per session. A headless `herdr server --session X` creates one with no client ever attached (also #24) |
| 50 | A phone can raise a desktop toast | `notification.show {title, body}` | Accepted — useful for "I'm taking this pane" and for pairing confirmations |

> **Named session vs workspace — do not conflate them.** A *named session* is a whole separate Herdr
> server with its own socket, created by the CLI and absent from the socket API. A *workspace* is the
> grouping inside one session, and `workspace.create` is an ordinary RPC. Kampr surfaces both, and
> creating a named session is the one management action that shells out rather than calling a method.


## Client rendering (Compose Multiplatform)

Raw logs behind every number: `client/terminal-spike/results/`. Each line carries the device string,
300 measured frames after 90 warm-up, and `draw_p50 / dropped / cache_hit_pct`.

| # | Claim | How | Result |
|---|---|---|---|
| 56 | **A 74×30 cell grid holds 60 fps on wasm and Android** | 26-scenario bench, realistic workload (~40 rows/s of patches, full reset every 2 s, blinking cursor) | wasm **draw 0.6 ms**, Android emulator **1.77 ms**, desktop 0.26 ms — **0 dropped of 300** everywhere. 200×50 also holds: 0.8 / 3.2 / 0.48 ms |
| 57 | The wasm backend is **real WebGL2**, not a Canvas 2D fallback | runtime DOM probe emitted at the head of every run | `canvas0=webgl2 renderer=ANGLE (Intel, Mesa Intel UHD Graphics)`. `navigator.gpu` exists in Chromium 151 but Skiko does not use it, and CMP 1.11's notes mention no WebGPU path |
| 58 | **Fill is free; text shaping is the entire cost** | differential measurement, 74×30 draw p50 | backgrounds only **0.0 / 0.04 ms**; shape every run every frame **8.8 / 14.17 ms**; **cached run layouts 0.6 / 1.77 ms**. Recomposition and buffer diffing are both within noise — `drawBehind`-vs-recomposition is not a lever worth engineering |
| 59 | Run-layout caching is **required**, not an optimisation | same | Shaping every frame is ~53 % of budget on wasm and ~85 % on the Android emulator; at 200×50 it drops 16–35 % of frames. Hit rate in practice: **99.2 %** — terminal output repeats run strings far more than intuition suggests |
| 60 | **Zoom must layer-scale during the gesture** and re-shape on settle | zoom-redraw vs `graphicsLayer` scenarios | Re-shaping at intermediate zooms collapses hit rate to ~51 %: wasm 11.6 ms / 8.5 % dropped at 200×50, Android 14.09 ms / 2.9 %. Layer scaling is 0.6–3.3 ms, zero drops. Pan alone is free either way |
| 61 | **A hand-rolled glyph atlas is a negative result on Skia** | rasterise each (glyph, style) into an `ImageBitmap`, blit per cell with `ColorFilter.tint` | Android **2.53 ms — the fastest mode measured**. But JVM desktop **192.8 ms/frame (2.2 fps)** and wasm **`RuntimeError: Aborted()` from Skia every frame**. Rendering was visually correct in all three, so this is the API path, not the idea: `DrawScope.drawImage` is cheap on Android and catastrophic on skiko at ~2 200 calls. **Skia already keeps its own GPU glyph atlas** — reach it from common code with per-glyph `drawText`, do not hand-roll one |
| 62 | Per-glyph `drawText` is the churn escape hatch | cache-hostile worst case, 74×30 | Holds 60 fps / 0 dropped where cached runs fall to 30 fps / 46–57 % dropped. So: cache runs by default, fall back per-glyph when a frame's hit rate collapses |

> **Android is emulator-only.** x86_64 on a desktop i7 with GPU passthrough, so the CPU-bound shaping
> cost is optimistic for a mid-range ARM phone. Even allowing a further 3×, 74×30 sits at ~5 ms of a
> 16.67 ms budget. **Re-measure on a real device before treating the headroom as banked.**

### Integration hazards found on the way

| # | Hazard | Consequence |
|---|---|---|
| 63 | **AGP 9 forbids `com.android.application` alongside the KMP plugin** | Shared code must use `com.android.kotlin.multiplatform.library`; the APK needs a separate plain-Android module. `client/` is laid out that way |
| 64 | **`compose.components.resources` emits no Android assets for a KMP-library target on CMP 1.11.1** | Fonts were prepared for jvm/wasmJs and **silently omitted from the APK** — Android fell back to system sans and rendered a proportional grid, so the first run measured the wrong font. Caught by screenshotting, not by the numbers. Needs a staged-assets task, and it will bite design tokens and icons too |
| 65 | **Resource fonts load asynchronously and can beat the first cell-metrics probe** | Column pitch computed from a font that is not what gets drawn, with nothing to recompute it. Gate first paint on font resolution |
| 66 | Ligatures desynchronise the grid | JetBrains Mono's `->`, `!=`, `==` collapse two cells into one glyph inside a shaped run. **Use the no-ligature cut** |
| 67 | This machine's `GRADLE_HOME` points at a broken install | `./gradlew` fails with `Cannot find module 'gradle-public-api-legacy'` unless invoked as `env -u GRADLE_HOME ./gradlew` |


## First end-to-end run (real client → real node → real herdr)

Browser client (Chromium 151, wasm bundle served by the node) against `kampr serve` against a
headless `herdr server --session kamprtest`. Everything below was seen live, not in a fixture.

| # | Claim | How | Result |
|---|---|---|---|
| 68 | **In a headless session the PTY does not follow the layout rect**, and `observe --cols <rect>` then crops the pane | created a workspace (rect 94×40), split it (rects 47/47), ran `stty size` in the left pane through Kampr | `40 93`. The PTY stayed 93 columns while the rect said 47, so the node observed at 47 and the remote view lost the right half of every row. Nothing in the socket API reports a pane's true PTY width — `pane.get` carries `viewport_rows` and no columns — so a node cannot detect the divergence. **The last sentence is wrong — corrected by #84/#85**, which measure the PTY width directly from `pane.read` |
| 69 | The rect is one column wider than the PTY even before a split | same session, single pane | rect `width: 94`, `stty size` → `93`. `observe` pads the missing column rather than cropping, so it is cosmetic where #68 is not. Fixed by the same measurement (#85): the node now streams at 93 |
| 70 | **A herdr server restart keeps its workspaces and panes** | `herdr server stop` under a live watcher, restarted 8 s later | Both panes came back (fresh shells, same ids). The node reconnected on its own and re-emitted `grid.reset`; no client action was needed. But **nothing told the client herdr was gone** — no `herdr_unavailable`, and `herd.nodes[].online` stayed `true` for the whole outage. **Fixed** — the outage flips `online`, emits `herdr_unavailable` and `node_offline`, and re-sends the whole `herd` on recovery |
| 71 | The documented scrollback gap-discard fires in ordinary use | `seq 1 40000` in a watched pane, 3 s poll | First `scrollback` 962 rows `capped:true`; the next read shared no overlap, so the ring restarted and `from_top` advanced to 963. A single verbose command is enough to lose history — #51's cap is not a corner case |
| 72 | Claude answers both its trust prompt and a tool-permission prompt on the **bare digit** | answered `1` from the Kampr UI to "Is this a project you trust?" and to a real `Bash` permission prompt | Both took effect with no submit key, including the dialog whose footer reads "Enter to confirm". Confirms #43 for Claude against two real dialogs |
| 73 | Herdr's own `observe` children outlive a node that is killed | `SIGTERM` to `kampr serve` with a pane watched | `herdr terminal session observe` survives, reparented to init, still streaming into a closed pipe. `kill_on_drop` cannot help — a signal skips the destructor |
| 74 | Backpressure never engaged against a stalled client | paused the client's TCP socket for 10 s while 3 000 lines were produced | 254 `grid.patch` against 255 for an unstalled client, and the 256-frame queue never overflowed — herdr's coalescing keeps the frame rate an order of magnitude below the bound |

## Transcripts, live

Against `claude` 2.1.237 in a headless `herdr server --session kamprconvo`, driven over the socket.

| # | Claim | How | Result |
|---|---|---|---|
| 75 | **Herdr never populates `pane.agent_session` for a detected harness** | ran a real `claude` in a headless session, then `session.snapshot` and `pane.get` across four turns | `agent: "claude"` throughout, `agent_session: null` throughout. The remote detection manifest (`~/.local/state/herdr/agent-detection/remote/claude.toml`) is **screen-scraping rules only** — regexes over the OSC title and the bottom lines. `pane.report_agent_session` exists as an RPC and nothing calls it. **So a node cannot reach a transcript through the session announcement; the pane's `cwd` is the only handle it gets**, and `agent_session` is a path that has to exist without ever firing |
| 76 | A Claude project directory is its cwd with every `/` replaced by `-` | compared 14 real project directories under `~/.claude/projects` against the `cwd` their records declare | Exact substitution, case preserved, `.`/`_` untouched: `/tmp/claude-1000/-home-dbrain/x` → `-tmp-claude-1000--home-dbrain-x`. Useful as a *hint* only — the transcript's own `cwd` is what proves a match, so the rule changing costs a slower search rather than the wrong conversation |
| 105 | Claude's trust prompt renders its docs link as an OSC 8 label | `pane.read visible strip_ansi` while held at the prompt | The line reads `Security guide` — no URL. It is the nearest non-empty line above the options, four non-blank lines below the real question, which is exactly what a "nearest line above" rule publishes. Reconfirms #36 from the other direction: the label survives, the URI does not |
| 106 | Answering on the **bare digit** still holds on 2.1.237 | answered `1` to the trust prompt and to a real `Bash` permission prompt, both through `pane.send_text` | Both took effect with no submit key, including the Bash dialog whose footer reads `Esc to cancel · Tab to amend`. Reconfirms #72 |
| 79 | **A CR in the same write as the text does not submit Claude's prompt box** | one `input {text: "…\r"}` through Kampr | The text landed in the box and stayed there. The same text followed by a *separate* `Enter` submitted immediately. Claude coalesces a burst into a paste, so a client must send the newline as its own message — the same reason `answer` sends a harness's submit key as a second write |

## Plugin install path (real herdr 0.8.2, isolated `XDG_CONFIG_HOME`)

A throwaway herdr home and session, a plugin root holding only `herdr-plugin.toml` and `packaging/`,
and a stub `kampr` served from a `file://` release. Nothing touched the operator's own herdr.

| # | Claim | How | Result |
|---|---|---|---|
| 80 | `herdr plugin link <dir>` registers a local plugin with no fetch and no `[[build]]` run | linked a plugin root whose `bin/` was populated by hand | All 7 `[[actions]]`, the `[[panes]]` popup and the `[[startup]]` hook registered. `plugin_root` is the linked directory itself, so `$0`-relative path resolution in the scripts is correct. This is the way to test action wiring without a release |
| 81 | Herdr injects `HERDR_PLUGIN_CONFIG_DIR` per plugin, under its own home | invoked the `url` and `status` actions and read `herdr plugin log list` | `/…/herdr/plugins/config/kampr`, distinct from the plugin root — the value `kamprctl.sh` passes to `--config-dir`. Both actions exited 0 and their stdout came back in the log |
| 82 | **A plugin action's exit code is the script's, and a failed `systemctl` inside one fails the action** | `status` on a host with no `kampr.service` | `systemctl --user status` exits non-zero, which took the whole action with it until `svc status` was made best-effort. `stop` and `uninstall` had the same shape — a stop of a unit that was never installed is the *normal* uninstall case |
| 83 | A herdr socket path over ~100 bytes is unusable | ran the isolated server under the agent scratchpad (`/tmp/claude-1000/-home-dbrain-dev-kampr/<uuid>/…`) | `local socket name length exceeds capacity of sun_path of sockaddr_un`. `sun_path` is 108 bytes, and `<home>/herdr/sessions/<name>/herdr.sock` eats the rest. Any test harness that relocates `XDG_CONFIG_HOME` must keep it short |

## The PTY's real width (headless `herdr server --session kampr-probe-w1/w2/w3`)

#68 concluded that a node cannot detect the rect/PTY divergence. That is wrong, and these four
rows close it.

| # | Claim | How | Result |
|---|---|---|---|
| 84 | **`pane.read` renders at the true PTY width, not the rect** | printed 400 `#` into a headless pane, read `visible`, then split and read again | Unsplit: rect 94, longest rendered row 93. After the split: rect 47, longest rendered row **still 93**, with `stty size` confirming `40 93` both times. The rect is fiction in a headless session and the reads are not — so the width **is** observable, and #68's "a node cannot detect the divergence" is disproved |
| 85 | **`recent` against `recent_unwrapped` gives the width exactly, not as a bound** | the same pane, `lines: 40`, both sources | `recent` → row lengths `[5, 68, 93, 93, 93, 93, 28, 34]`; `recent_unwrapped` → `[5, 68, 400, 34]`. A logical line longer than the widest physical row proves a soft wrap, and a soft wrap happens at exactly the PTY width. Without a wrap the reads are identical and the widest row is only a **floor** — herdr trims each row's trailing blanks, so a screen holding a 44-column prompt reads 44 |
| 86 | `recent` at `lines == viewport_rows` is safe on a *detected agent* pane | `pane.report_agent` on a pane holding `seq 1 500`, then both reads at `lines: 40` against `viewport_rows: 40` | 18 ms each, `offset_from_bottom` and `max_offset_from_bottom` unchanged at `0`/`463`. #27's harvest hazard is `lines > viewport_rows`; the equality case is the viewport and does not move the operator's screen. This is what lets the exact measurement apply to agent panes too |
| 87 | **`observe` at more than the PTY width pads; at less it crops** | observed the same split pane at `--cols 47`, `94` and `200` | `#` run lengths `47` / `93` / `93`. The reported `width` in the frame record just echoes the request, so it carries no information — but over-observing is lossless and reflows nothing, which is what makes "never narrower than the measured width" a safe rule |

## What a program can learn about its background (headless `herdr server --session kampr-ground`)

| # | Claim | How | Result |
|---|---|---|---|
| 88 | **Herdr answers no `OSC 11` / `OSC 10` colour query, and sets no `COLORFGBG`** | a bash script in a headless pane wrote `ESC ]11;? ESC \` then `ESC ]10;? ESC \`, reading the PTY back with a 2 s timeout after each; `CSI c` in the same script as a control | Both OSC queries returned **0 bytes**. The control answered `ESC[?62;22c` immediately, so the PTY does reply to queries — it just does not implement these two. `COLORFGBG` is unset; `COLORTERM=truecolor`. **A harness in a herdr pane cannot discover what background it is on, and is told it has 24-bit colour** — so it authors for dark and in absolute values, which is the whole basis of [ADR 0009](./adr/0009-the-terminal-keeps-its-own-ground.md). Independently reached by Collie, whose ADR 0002 measured the consequence downstream |

## Driving every management op from the client (throwaway `kampr-it-clientops-*`)

| # | Claim | How | Result |
|---|---|---|---|
| 89 | **A pane herdr has just created is not immediately an agent target** | `pane.split`, then `agent.start {kind:"claude"}` on the new pane as fast as the ack came back | `agent_pane_busy: agent target pane w2:p4 is not an available shell`, then success on a retry a few hundred ms later. Every other op lands on a new pane at once — this is the one that has to wait for the *thing* rather than for its own answer, so a client either retries or offers the action only once the pane reports a shell |
| 90 | **`layout.apply` renumbers the tab's panes** | `layout.export` a two-pane tab, `layout.apply` the same tree back unchanged, then `close` a pane id held from before | `pane_not_found: pane w2:p4 not found`. A round-trip that changes nothing structurally still rebuilds the panes under new ids, so any id a client is holding across a `layout.apply` is stale. The `herd.patch` that follows is the only source of the new ones |
| 91 | **A pane id carries its workspace but never its tab** | `session.snapshot` ids across a workspace with two tabs | `w3:p2` yields `w3` by construction, and nothing yields `w3:t1` — which is why `tab.rename` / `tab.close` / `tab.focus` were unreachable from a client until `herd.panes[].tab_id` was added to the wire |

## What a screen reader is handed (TalkBack on an API 35 emulator, against a live herdr pane)

| # | Claim | How | Result |
|---|---|---|---|
| 92 | **A key cap driven by `detectTapGestures` is reachable, unnamed and impossible to press** | TalkBack on, linear navigation across the pane screen, then double-tap on two caps | Before: every cap announced as an unlabelled control and a double-tap did nothing — TalkBack's activation dispatches the *semantic* click, which a raw pointer handler never receives. After adding `onClick`/`onLongClick` semantics: double-tapping the caps named "Slash key" and "Hyphen key" put `///---` into the shell, and the cursor-line live region reported it back. This is the defect that made the key row decorative rather than merely unlabelled |
| 93 | **Reading a viewport aloud is minutes of speech per repaint** | the probed pane at its two widths, against TalkBack's default rate (~200 wpm ≈ 16 chars/s) | 94×40 is 3,760 characters ≈ **4 minutes**; after the desk resized it, 206×40 is 8,240 ≈ **9 minutes**. `revision` increments per `grid.patch` and #56 holds 60 fps, so a live region over grid contents restarts that utterance sixty times a second. The basis of [ADR 0010](./adr/0010-the-grid-is-described-not-read-out.md) |
| 94 | d-pad traversal on the herd follows reading order and skips static text | TalkBack on, `KEYCODE_DPAD_DOWN` from the top of the herd screen | Node count pill → mosaic → the pane card ("Open ~ · bash, No status, /home/dbrain, updated 6m") → bottom navigation. The node header is spoken by linear (swipe) navigation but is not a d-pad stop, because a d-pad walks focusable controls — standard, and worth knowing before reading a traversal trace as a defect |
| 95 | **`agent.start` argv reaches the harness end to end from the client** | Kampr's New sheet in a real browser against a node on a throwaway herdr session; typed `--dangerously-skip-permissions`, pressed Start claude, then `pane.read` | Claude Code v2.1.238 came up with `⏵⏵ bypass permissions on` in its footer. The node has always forwarded `args` (`manage.rs:186`); the client had simply never set them. A shell alias is not needed — the argv behind one is what `agent.start` takes |
| 96 | **The wire `prefs` blob cannot hold a setting that is not about a pane** | `prefs` write carrying an id the node is not serving | `unknown_pane` (`session.rs:746`). Every stored key sits under a pane id, so a per-agent-kind or per-device preference has no home on the wire and belongs in the client's own prefs |
| 97 | **A kampr node serves every herdr session it can find, not just its configured socket** | `kampr serve` with `[herdr] socket` pointed at one named session | It also discovered `~/.config/herdr/herdr.sock` and published it as a second node. `[herdr] sessions = ["…"]` is what confines it — worth knowing before any test drives a herd on a machine with live work on it |
| 98 | **`kampr pair` refuses to arm a code without a TTY once any device is enrolled** | ran it non-interactively after one browser had paired | ``Run `kampr setup` in a shell to allow a device to pair.`` The arming keypress (`pairing.rs:26`) needs both stdin and stdout to be terminals; driving it from a test means a pty, and the window is 60 s |
| 99 | **Compose Multiplatform/wasm publishes no accessibility DOM unless assistive tech is detected** | headless Chromium, `document.querySelector('#cmp_a11y_root')` after the app had painted and was interactive | Zero nodes, on a page whose controls all carry `Modifier.action` names. The a11y mirror is not a reliable probe for browser-side assertions; the JVM `runComposeUiTest` semantics tree is where label-level checks belong. Corrects the method used by #92–#94 |
| 100 | **Android 17's local-network restriction keys on *on-link* addresses, not RFC1918 membership** | targetSdk 37 on a stock API 37 image, no compat override; requested `10.0.2.2:8793` (on-link) and the host's LAN `10.0.0.163:8793` (routed via the gateway) with and without `ACCESS_LOCAL_NETWORK` | On-link without the permission **times out**; on-link with it, 200. Off-link answers 200 either way. From an emulator the host's LAN IP is *not* classified local, so testing only that address is a **false pass** — on a real phone sharing Wi-Fi with the node it is on-link and would be refused |
| 101 | **A missing `ACCESS_LOCAL_NETWORK` is invisible** | same setup, watched logcat and the client's own error path | The refusal surfaces as a 10 s `SocketTimeoutException`, never a permission error. An undeclared permission is indistinguishable from a node that is down — which is why it is worth a manifest-level test rather than a runtime check |
| 102 | **Instrumentation cannot enable a compat change on itself** | `am compat enable RESTRICT_LOCAL_NETWORK dev.kampr.app` from inside a test | PlatformCompat kills the named process. The durable guard has to assert the manifest declaration; forcing the real refusal means enabling the change from a shell outside the app |
| 103 | **cmdline-tools 23 retires `sdkmanager` and cannot install several packages at once** | installed platform 37, build-tools 37.0.0, platform-tools, emulator and a system image | `sdkmanager` prints a deprecation and delegates to a new `android` CLI. `android sdk install` fails with a `Storage.saveArchive` stack trace on a multi-package invocation; one package per call works. API 37 platforms are named `android-37.0`/`android-37.1`, not plain `android-37` |
| 104 | **Gradle 9.7.1 makes toolchain auto-provisioning without a repository a Gradle-10 error** | `./gradlew build` on the wrapper bump | Cleared by `org.gradle.toolchains.foojay-resolver-convention` in `settings.gradle.kts`. The remaining Gradle-10 blocker is `co.uzzu.dotenv` 4.0.0 calling `Project.getProperties`; 4.0.0 is the latest, so there is nothing to bump to |
| 108 | **A screen-reader review mode over the grid is reachable and drivable under TalkBack** | API 37 emulator, TalkBack on, live `kampr serve` against a real herdr pane; `uiautomator dump` after each step | The grid offers `Review this pane row by row`; seven named controls; the readout is a polite live region carrying `row N of M. <text>`; eight taps on *previous row* moved 99→91 exactly, and TalkBack took accessibility audio focus per step |
| 109 | **A repaint under the review cursor moves nobody and speaks once** | parked on row 96 of 100, then wrote to the pane from herdr | The cursor stayed on 96; one notice node, `The pane wrote to the row you are reading.`; the next *read again* answered `Changed. row 99.`; leaving review restored the cursor-line region. Re-reading on every repaint would be the same firehose [ADR 0010](./adr/0010-the-grid-is-described-not-read-out.md) exists to refuse, narrowed to one row |
| 110 | **A Compose live region will not re-speak identical text** | re-read the same row twice under TalkBack | Silent unless the description changes. An unspoken no-break space on alternate reads forces it, and TalkBack does not voice it. This is a trick, and it is load-bearing for *read this row again* |
| 111 | **The node's `capped` / `complete` reach the terminal surface** | `LiveScrollbackTest` against a real node: one pane filled past herdr's 1000-line cap before watching, one watched from empty then `seq 1 9000` to outrun the poll | Clipped ring → *herdr caps a read at 1000 lines*; intact ring → **nothing at all**; gap → `N rows were discarded` carrying the node's own `from_top`. With the surfacing reverted the same test fails against the live node with *"a clipped ring said nothing"* |
| 112 | **A ring that restarts on a width change has to adopt the new width, or it restarts on every read for ever** | reproduced live at `scrollback_max_rows = 60`: short output first so the ring is keyed on the rect, then a wrapping line, logging `raw.cols` beside `from_top` per read | The width moves **once**, 94→93, the instant a soft wrap proves the PTY is a column narrower than its rect (#69). `ScrollbackRing::restart` took the new rows and kept the old `cols`, so every read after it disagreed with the ring too: `dropped=60` per read and `from_top` climbing ~285 rows/s **on a pane that had gone quiet**, history pinned at one read's depth, `capped` permanently true, and the whole ring re-sent to every client every 3 s. Neither original hypothesis held — ordinary trimming logs nothing at all, and the replies need only disagree *once*. Fixed; regressions at the ring, at the registry and end-to-end, the last asserting an idle pane's `from_top` stops moving |
| 113 | **Credential Manager signs `android:apk-key-hash:<b64url>`, not an `https://` origin** | read `webauthn-rs-core`'s origin check (exact match for opaque URLs, `core.rs:484`); derived the origin and cross-checked it against an independent worked example | An RP that does not allow that origin refuses **every** native ceremony — *after* the owner has already approved it on screen. `assetlinks.json` alone is not enough; the node must add the apk-key-hash origin to its WebAuthn engine |
| 114 | **`webauthn-rs`' generic passkey options are a ceremony Android cannot perform** | captured `register/start` from a live node with and without `platform=android` | Generic gives `residentKey: discouraged`, no attachment and `credProtect`; Android needs discoverable, platform, no credProtect. The crate ships `workaround-google-passkey-specific-issues` for exactly this. A browser's option set must stay byte-for-byte unchanged |
| 115 | **Google's own Digital Asset Links service parses the node's file** | node exposed on a temporary public HTTPS tunnel, `digitalassetlinks.googleapis.com/v1/statements:list` with `relation=…get_login_creds` | Both delegations returned with the correct package and fingerprints — validated by the service Android itself relies on, rather than by our own reading of the format |
| 116 | **A stock AVD with no Google account has no credential provider** | tier-1 node, screen lock set, "Add a passkey" on a stock API 37 `google_apis_playstore` image | GMS answers `No create options available`. Everything up to the provider can be verified on an emulator; passkey **creation** cannot. It has therefore never been done anywhere — run it once against a real phone |
| 117 | **The emulator's virtual scene cannot put a QR in front of the camera** (API 37, emulator 37.1.11) | `-virtualscene-poster` at launch and `adb emu virtualscene-image wall/table`, 1-bit and 24-bit PNG, swept over 9 positions × 12 yaws × 3 pitches via the emulator's gRPC `setPhysicalModel` | No visible change anywhere in the scene — screenshot diff 0.4% AE, i.e. sensor noise. Separately: `screencap` renders a CameraX `PreviewView` **black while the camera is streaming**, so a black preview in a screenshot is not evidence of a broken camera |
| 118 | **Compose UI tests are structurally blind to system-bar insets** | 456 client tests passing while the gesture handle sat on top of the "Pane" label on a real device | Insets are zero in a test window, so no semantics-tree assertion can see them — and a fixed-height box taller than the test window passes **vacuously**. What catches it is a composition local (`LocalSafeArea`) a test can *provide*, asserted against `onRoot()` bounds rather than a constant |
| 119 | **The gesture inset moves to the *side* in landscape, and only three-button navigation reveals it** | rotated a gesture-nav API 37 AVD, then `adb shell cmd overlay enable com.android.internal.systemui.navbar.threebutton` | Rotating a gesture-nav AVD is a **false negative** — `systemBars` reports bottom-only in both orientations. Three-button nav is what makes left/right non-zero, and the first screenshot after enabling it showed the herd screen's right-hand column drawn under the navigation cluster. A defect nobody had looked for |
| 120 | **A top-inset assertion is vacuous in the mirror image of the way a bottom-inset one is** | wrote the obvious top assertion, watched it pass before the fix | [#118](#)'s trap was a fixed height taller than the test window. The top trap is measuring a node that was never at the top: anything below a header's first row clears a 32 dp bar whichever way the code goes. The assertion has to name the **topmost** node and fail if it finds none |
| 121 | **Two notions of "how much room the chrome takes" is the bug, not one** | reverted the `MosaicSwitcher` chrome constant while leaving its bar padded | `PaneScreen` hands the terminal a *measured* chrome height; `MosaicSwitcher` hands it a *constant*. Padding the bar without growing the constant leaves the top grid row behind the bar **with no scroll left to reach it** — invisible to any test that only checks the controls moved. Caught by asserting the terminal's own inset against the bar's measured bottom in one test |
| 122 | **A healthy node can serve an unreachable `/.well-known/assetlinks.json`** | added a `doctor` check that asks the *origin* rather than the config | The node always builds the document correctly, so the failure mode is never the file — it is a proxy with its own `/.well-known` location block. That reads as a perfectly healthy node right up to the moment Credential Manager refuses the ceremony the owner has already approved on screen |
| 123 | **`co.uzzu.dotenv` is replaceable by ~35 lines of settings plugin** | five TestKit builds reproducing the kobup helper's `try { env.fetchOrNull(…) } catch` shape verbatim, plus an init-script probe of the real `:androidApp` extension | The helper's whole contract is `fetchOrNull(String)`. `gradle.lifecycle.beforeProject` puts `env` on every project without `allprojects`, and `providers.fileContents`/`environmentVariable` keep it configuration-cache-correct. `.env` → value, exported variable → value, neither → `null`. This is what cleared the last Gradle 10 blocker |
| 124 | **A new Gradle included build re-introduces the toolchain deprecation on its own** | `./gradlew test --warning-mode all` inside the new included build, before and after | `jvmToolchain(21)` auto-provisioned without a repository and printed the Gradle-10 error warning until the *included build's own* settings got the foojay resolver. The convention does not inherit |
| 125 | **A refused write is invisible in the audit log** | read-only token on `/ws`; `input` and `manage{pane.close}` at a pane id no herd resolves, so the role gate runs before pane resolution and nothing real is touched; `audit.jsonl` diffed | Both refused correctly and **recorded nothing**. The `manage` audit happens *after* the role gate, so refused ops are unlogged too. Everything else in this node is audited thoroughly, which is what makes this the one invisible thing |
| 126 | **`[herdr] sessions = []` is the only way to stop a node enumerating the whole host** | `/api/warm` against a live node with the key absent, then present | With the key absent, a node pointed at one session's socket still listed **every** herdr session on the machine as its own node. An empty list serves only the configured one; an absent key serves everything |
| 127 | **`live.rs::resync_repaints_every_watched_pane_and_unwatch_stops_one` is contention-flaky** | three full-suite runs at `--test-threads=2` and three in isolation, against real herdr 0.8.2 | Fails roughly one run in three with *"an unwatched pane must stop streaming"* (left 1, right 0); 3/3 green alone. Timing, not the code under test — but it is the only suite in the tree that does this, so it is worth knowing before chasing it as a regression |
| 128 | **A UI transient cannot be observed in `runComposeUiTest` with the default clock** | asserted a self-clearing notice appears, is a polite live region, then goes | `waitUntil`/`waitForIdle` advance the main clock while a `withFrameNanos` loop keeps the composition non-idle, winding straight past a 6 s lifetime to expiry **before any assertion runs**. `mainClock.autoAdvance = false` plus explicit `advanceTimeBy` is the only way. Any test of a thing that disappears needs this |
| 129 | **The pane's meta line has no room for one more fact** | 411×914 portrait, real bundle, headless Chromium | It ellipsises at `… · observ…`, so a tag appended after `observing` was invisible in every screenshot. Readable only when it **replaces** the constant — which is the better trade anyway: `observing` never changes, and the tag is the half that is news |
| 130 | **`pane.output_changed` is emitted but not subscribable, and it takes the whole list with it** | offered each of 25 candidate kinds to herdr 0.8.2 individually | 24 accepted; `pane.output_changed` refused with `invalid_request: unknown variant`, and herdr names its own 27 subscribable kinds in the error text. A subscription list is all-or-nothing, so including it costs **every other subscription** and drops the node silently back to its sweep with nothing in the log. The schema file lists it under `event` — that catalogue is what herdr *emits*, not what it lets you subscribe to, so no schema check substitutes for offering the real list to a real herdr |
| 131 | **`events.subscribe` replays the whole herd as a `created` burst the instant it opens** | subscribed to 24 kinds against a session with one workspace; traced calls through a counting socket proxy | 8 events at t=0.00 and none after, including through `echo` and `seq 1 300` — **terminal output produces no subscribable event at all.** The node's 60 ms "settle" slept and refreshed *per event*, so every subscribe cost ~8 `session.snapshot` calls. Sleeping, then draining, then taking one snapshot makes it one |
| 132 | **A `scrollback_max_rows` below herdr's ~1000-row read makes every read an ungappable `Gap`, and the poller answers by going to 10 Hz forever** | observed at `scrollback_max_rows = 60` while diagnosing #112 | `overlap()` matches held-suffix against incoming-prefix only; when the ring holds fewer rows than one read returns, the held rows sit at the *end* of incoming and overlap is always 0. Each gap sets the cadence to `HistoryPolicy::fastest` (100 ms), so a quiet watched pane issues ~10 `pane.read` calls a second indefinitely. **Not fixed** — the correct stitch is a substring match, and the ring discards rather than splices on purpose so absolute indices stay true. Needs a non-default config to trigger (default 20 000). A safer partial mitigation is to stop treating a structural gap as a reason to poll faster, since polling faster cannot help |

## Still open

- Does `pane.read recent` with `lines > viewport_rows` scroll a pane with a *detected agent*?
  (#27 covered the no-agent case; #86 covers the equality case. The interlock still assumes the
  worst above the viewport.)
- **A real mid-range ARM phone.** #56 is emulator-only; the shaping cost is the part that will move.
- Why herdr's headless rect is one column wider than the PTY it created (#69). It no longer
  matters — #85 measures the PTY directly — but nothing explains the reserved column.
