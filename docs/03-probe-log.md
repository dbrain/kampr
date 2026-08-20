# Probe log

> **Probe numbers are permanent identifiers, not positions.** Code and docs cite them, so renumbering
> silently repoints a citation at unrelated evidence — which has happened once already: four sites
> citing #75 for the PTY/rect divergence ended up pointing at an `agent_session` finding after a
> renumber. Append new rows; never renumber an existing one. If a probe is superseded, strike it
> through and reference the row that replaced it.


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
| 77 | Claude's trust prompt renders its docs link as an OSC 8 label | `pane.read visible strip_ansi` while held at the prompt | The line reads `Security guide` — no URL. It is the nearest non-empty line above the options, four non-blank lines below the real question, which is exactly what a "nearest line above" rule publishes. Reconfirms #36 from the other direction: the label survives, the URI does not |
| 78 | Answering on the **bare digit** still holds on 2.1.237 | answered `1` to the trust prompt and to a real `Bash` permission prompt, both through `pane.send_text` | Both took effect with no submit key, including the Bash dialog whose footer reads `Esc to cancel · Tab to amend`. Reconfirms #72 |
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

## Still open

- Does `pane.read recent` with `lines > viewport_rows` scroll a pane with a *detected agent*?
  (#27 covered the no-agent case; #86 covers the equality case. The interlock still assumes the
  worst above the viewport.)
- **A real mid-range ARM phone.** #56 is emulator-only; the shaping cost is the part that will move.
- Why herdr's headless rect is one column wider than the PTY it created (#69). It no longer
  matters — #85 measures the PTY directly — but nothing explains the reserved column.
