# Probe log

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


## Still open

- Does `pane.read recent` scroll a plain shell pane *that has a detected agent*? (#27 covered the
  no-agent case; the interlock assumes the worst for the other.)
- **A real mid-range ARM phone.** #56 is emulator-only; the shaping cost is the part that will move.
