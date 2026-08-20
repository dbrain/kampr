# Kampr

Remote access to a [Herdr](https://herdr.dev) agent herd from a phone, a tablet or a browser —
across several machines, behind real authentication, without reshaping the session you left running
at your desk.

**Status: pre-alpha. Do not expose this to a network yet.** The node, clients, transcripts and
packaging are built and tested in isolation; a completeness audit found real defects at the seams
between them, and a security pass is in flight. See `docs/06-audit.md` for exactly what is broken.

## What it is

- **A live terminal, not a text mirror.** Kampr streams Herdr's own rendered frames and reconstructs
  the grid exactly — truecolour, cursor, hyperlinks and all.
- **A conversation view for agents.** Claude and Codex transcripts render as real markdown, so a
  table is a table. This is the default view for agent panes.
- **Many machines, one herd.** *Planned — the mesh is designed and not yet built; a node currently
  serves its own host only.*
- **It never resizes your panes.** Kampr observes and types; it structurally cannot reshape a
  session. Small screens are handled by zoom, pan and the conversation view.
- **Runs from a bare IP and port**, with certificates, passkeys, notifications and a mesh as optional
  rungs you opt into.

## Layout

| Path | |
|---|---|
| `crates/kampr-herdr` | Herdr socket client, snapshot model, `observe` supervisor, scrollback |
| `crates/kampr-term` | VT emulation → cell grid, dirty rows, OSC 8 |
| `crates/kampr-core` | Provider seam, pane registry, one emulator per pane, wire encoding |
| `crates/kampr-node` | axum server, `/ws`, herd model, `manage` ops |
| `crates/kampr-auth` | Tiers, devices, tokens, passkeys, audit log |
| `crates/kampr-journal` | Claude and Codex transcript adapters |
| `crates/kampr-cli` | The `kampr` binary — `init`, `serve`, `setup`, `service` |
| `crates/kampr-spike` | End-to-end fidelity check against Herdr's own grid |
| `client/shared` | Theme tokens, WS client, herd model, navigation |
| `client/terminal` | Cell renderer, zoom and pan, key row, input capture |
| `client/conversation` | Markdown, tables, tool cards, diffs |
| `client/{android,desktop,web}App` | Composition roots |
| `docs/01-implementation-findings.md` | What Herdr exposes and what is possible |
| `docs/02-roadmap.md` | Tickable plan |
| `docs/03-probe-log.md` | Every claim about Herdr, traced to a command |
| `docs/04-wire-protocol.md` | The node ↔ client contract |
| `docs/05-agent-briefs.md` | Parallel workstreams |
| `docs/06-audit.md` | Completeness audit — what is broken, incomplete, and missing |
| `docs/design/` | Design canvas sources — themeable artboards |
| `research/` | Herdr API schema, method catalogue, mirrored docs, probe tooling |

## Check the pipeline

```bash
herdr --session probe                          # a throwaway session in one terminal
HERDR_SESSION=probe cargo run -p kampr-spike   # in another
```

Reconstructs the pane's grid from the frame stream alone and diffs it against Herdr's own
`pane.read visible`. It should print `PERFECT MATCH`.

## Requirements

Herdr **0.8.2+** (protocol 20), Rust 1.90+. Clients: Kotlin 2.4.10, Compose Multiplatform 1.11.1.

## Licence

MIT.
