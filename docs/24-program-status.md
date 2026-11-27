# Program status: adopting OSC 7501 properly

Superlogical's Rex defined [the Program Status Protocol](https://www.superlogical.com/rex/docs/build/program-status)
(`OSC 7501`): a program tells the terminal what it is doing, and the terminal keeps one record per
program. Claude Code already speaks it and a native session (docs/17) already answers it, so the channel
exists here as "Claude's own 7501" (docs/14, #600). What exists is a Claude-shaped subset. This brief
makes it the protocol, so that anything that adopts it (cargo, terraform, brew, a deploy script) shows up
in a native session's status with no per-program work.

## What we do today, against the spec

Read from `crates/kampr-native/src/answer.rs` (`program()`) and `kampr-core/src/provider.rs`
(`ProgramStatus`) on 2026-11-27; the spec was read from the Rex docs the same day.

| Spec | Kampr today | Gap |
|---|---|---|
| `OSC 7501;?` answered with `?` (optionally `states=`, `kinds=`) | answered with a bare `?` | none; add `states`/`kinds` only if we extend them |
| A malformed pair is skipped, the rest of the report is processed | `split_once('=')?` drops the **whole report** on one bad pair | wrong |
| Unknown keys ignored; a repeated key: last wins | same | none |
| Free text is base64 UTF-8 with no control characters, else the **whole report is discarded** | decoded lossily, control characters kept, a bad `msg` silently dropped while the report applies | wrong |
| Limits: 4096 B sequence, 2048 decoded `msg`, 192 `title`, 32 `app`, id 128 B / 32 per segment / 8 deep, 256 records | none enforced | needed: the node reads untrusted program output |
| **Records by `id`**: a report replaces its record whole; `/` makes parent and children; `clear` removes a record and everything below it; `clear` with no id removes all | any report with an `id` is **thrown away** (`program()` returns `None`); the one record is the root | the biggest gap. Dropping Claude's subagent reports was deliberate (#600); every other program's records go with them |
| `app` inherited from the nearest ancestor, `title`, `progress` 0–100 on working/blocked | parsed past and discarded | kept in the model, shown nowhere |
| `kind` only on `blocked`; an unknown kind is absent | kept on any state | ignore it elsewhere |
| Lifetime: `working`/`blocked` dropped on process exit or a new shell prompt (`OSC 133 A`); `done`/`error` survive both; no heartbeat | cleared by `state=clear` or the session's exit; `OSC 133` is not handled anywhere (grep) | a shell that runs a program reporting `working` and is killed leaves `working` for ever |
| `RIS` removes every record, `DECSTR` does not, an alternate-screen switch does nothing | not checked | check, then test |
| `error` is its own state | held as `ProgramState::Error`, shown on the wire as `idle` ("the wire has no error state") | decision below |
| Rate-limit anything a record causes outside the terminal | not checked for per-record causes | check when records can raise pushes |

## The model

A native session's emulator holds a **record set**, not an `Option<ProgramStatus>`. A record has `id`
(the path, empty for root), `state`, `kind`, `progress`, `app`, `title`, `msg` and the log offset of the
report that wrote it. The checkpoint carries the set (today it carries "the last report"), which is a
checkpoint format change: version it and keep reading the old.

The **session's status** is a fold over the set, so a program that only ever reports root changes nothing:

- `blocked` anywhere beats everything (a worker blocked on a permission is the session blocked, and its
  `kind`/`msg` is what the pending strip shows);
- else `working` anywhere, else `error`, else `done`, else `idle`;
- the root record alone supplies the harness's own idle/done (a worker finishing is not the session
  finishing).

Claude Code's subagent reports (`id=…`) then fold in instead of being dropped. **Measure first** (rule
2): `research/probe/osc7501-hosted.py` should record what current Claude sends while it runs a Task and a
background worker, and on interrupt, a permission dialog and `/clear`. #600's decision to drop ids was
made on one session's output; a probe row confirms the fold is right rather than assuming it.

## Wire (additive only)

`agent_status` keeps its values; `error` is not added to it, because an older client renders an unknown
status as blank. New optional pane fields:

- `program_error: true` while the folded state is `error`, with `program_msg`. A client that knows it
  marks the row; one that does not sees `idle`, which is what it sees today;
- `program: [{id, state, kind, progress, app, title, msg}]`, the record set, capped at 32 rows on the
  wire though the node holds 256; omitted when there is only a root record.

`wire-shapes.txt` gets the new paths (`KAMPR_BLESS_WIRE=1`) and docs/04 the fields.

## Where each piece goes

1. `kampr-core`: `ProgramStatus` becomes a `ProgramRecords` type (the set, the fold, the limits, the
   lifetime rules): pure, table-tested, no emulator. The strict parser lives here so one implementation
   serves the keeper's tail, `kampr attach` and a hosted session.
2. `kampr-native/src/answer.rs`: `osc_dispatch` hands the body over; `OSC 133 A` and process exit drive
   the lifetime rule; `RIS` clears.
3. `kampr-native` checkpoint: carry the set.
4. `kampr-node/src/state.rs`: the fold feeds the status ladder where `program` does now; push and triage
   read `blocked` records, so a worker's permission raises a push.
5. Client: the `program_error` row mark, a progress bar on a working row that has `progress`, a records
   list under a pane with more than a root. A UI decision: sketch it before building. Steps 1–4 do not
   wait for it.

## Herdr panes

herdr answers DA1 and not `7501;?` (#611), so Claude never starts reporting in a plain herdr pane. The
spec lets a program report without asking, and `kampr attach` already answers for a hosted session, so
found panes change nothing. If herdr ever relays 7501, the same parser applies.

## Tests (TDD, in this order)

1. A conformance table in `kampr-core`: every example on the spec page (one program, several records,
   `clear` of a subtree, `clear` of all) and every rejection (control character, bad base64, each limit,
   bad id grammar, unknown state) with the expected record set.
2. An emulator test per lifetime rule: `working` dropped on process exit and on `OSC 133 A`, `done` kept,
   `RIS` clears, an alternate-screen switch does not.
3. A checkpoint round-trip carrying several records, and a replay rebuilding the same set.
4. A node test that a worker's `blocked` makes the pane `blocked` and raises one push.
5. Mutation-check each: restore the defect, watch it fail (#191).

## Decisions for the operator

- Fold subagent records into the session, or keep ignoring them? (recommend fold, after the probe)
- Add `program_error`/`program`, or leave `error` as `idle` and ship only the parser hardening?
  (recommend the fields: it is the only way the phone ever says "failed")
- Per-record rows in the client, or only the fold plus a progress bar?
