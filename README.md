# Kampr

Remote access to a [Herdr](https://herdr.dev) agent herd from a phone, a tablet or a browser —
across several machines, behind real authentication, without reshaping the session you left running
at your desk.

**Status: design and foundations.** The Herdr integration and the terminal emulator work and are
tested; the server, clients and packaging are specified and not yet built.

## What it is

- **A live terminal, not a text mirror.** Kampr streams Herdr's own rendered frames and reconstructs
  the grid exactly — truecolour, cursor, hyperlinks and all.
- **A conversation view for agents.** Claude and Codex transcripts render as real markdown, so a
  table is a table. This is the default view for agent panes.
- **Many machines, one herd.** Nodes peer with each other; only the hub needs to be reachable.
- **It never resizes your panes.** Kampr observes and types; it structurally cannot reshape a
  session. Small screens are handled by zoom, pan and the conversation view.
- **Runs from a bare IP and port**, with certificates, passkeys, notifications and a mesh as optional
  rungs you opt into.

## Layout

| Path | |
|---|---|
| `crates/kampr-herdr` | Herdr socket client, snapshot model, `observe` supervisor, scrollback |
| `crates/kampr-term` | VT emulation → cell grid, dirty rows, OSC 8 |
| `crates/kampr-spike` | End-to-end fidelity check against Herdr's own grid |
| `docs/01-implementation-findings.md` | What Herdr exposes and what is possible |
| `docs/02-roadmap.md` | Tickable plan |
| `docs/03-probe-log.md` | Every claim about Herdr, traced to a command |
| `docs/04-wire-protocol.md` | The node ↔ client contract |
| `docs/05-agent-briefs.md` | Parallel workstreams |
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
