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
| 29 | Over-asking clamps harmlessly | `lines=5000` | Returns 400, `truncated:false`, viewport unmoved |
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
| 40 | Every `tool_use` in a finished session has a `tool_result` | same | 300/300 — proves nothing about whether a *pending* request is flushed before approval. **Still open** |

## End-to-end

| # | Claim | How | Result |
|---|---|---|---|
| 41 | **`observe` → emulator → cell grid reproduces herdr's own grid exactly** | `cargo run -p kampr-spike`, compared against `pane.read visible` | **30/30 rows identical**, cursor at the right cell, 1 hyperlink interned. Truecolour, 256-colour, reverse, underline, box drawing and non-ASCII all round-trip |

Reproduce #41:

```bash
herdr --session probe                        # in one terminal
HERDR_SESSION=probe cargo run -p kampr-spike # in another
```

## Still open

- Does `pane.read recent` scroll a plain shell pane *that has a detected agent*? (#27 covered the
  no-agent case; the interlock assumes the worst for the other.)
- Is a pending tool request in the transcript before approval? (#40)
- CMP wasm: 74×30 cell grid at 60 fps on a mid-range Android.
