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
- **Many machines, one herd.** Peers dial **outbound** to a hub, so a laptop behind NAT joins with
  no port forwarding and you point one reverse proxy at one hostname. Node-to-node auth is a mutual
  ed25519 handshake with an explicit join step, separate from any device token. See
  `docs/07-mesh-deployment.md`.
- **It never resizes your panes.** Kampr observes and types; it structurally cannot reshape a
  session. Small screens are handled by zoom, pan and the conversation view.
- **Runs from a bare IP and port**, with certificates, passkeys, notifications and a mesh as optional
  rungs you opt into.

## Install

Both routes need a tagged release to exist; until the first tag lands, build from source (below).

As a Herdr plugin — the turnkey path:

```bash
herdr plugin install dbrain/kampr
```

Standalone:

```bash
curl -fsSL https://github.com/dbrain/kampr/releases/latest/download/install.sh | sh
kampr init             # config, node keypair, URL + pairing code, QR
kampr service install  # systemd --user unit, launchd agent on macOS
```

Both paths run the same script and both refuse to install a binary they cannot verify. Every release
publishes `SHA256SUMS`, which is checked before anything is unpacked, and a keyless
[cosign](https://docs.sigstore.dev/cosign/installation) signature over it. Install cosign first and
the signature is checked too; without it the installer says so rather than pretending.

Linux binaries are statically linked against musl, so there is no glibc floor — an old Raspberry Pi
is fine. macOS needs 11.0 or newer. **Windows is not supported**: the node reaches Herdr over a Unix
domain socket and supervises itself with systemd or launchd. Use WSL2.

From source, note the build order — the node embeds the web client, so the bundle is staged before
cargo runs:

```bash
cd client && ./gradlew :webApp:stageNodeBundle && cd ..
KAMPR_REQUIRE_BUNDLE=1 cargo build --release -p kampr-cli
```

`KAMPR_REQUIRE_BUNDLE=1` fails the build rather than quietly producing a binary that serves the
placeholder page.

## Layout

| Path | |
|---|---|
| `crates/kampr-herdr` | Herdr socket client, snapshot model, `observe` supervisor, scrollback |
| `crates/kampr-term` | VT emulation → cell grid, dirty rows, OSC 8 |
| `crates/kampr-core` | Provider seam, pane registry, one emulator per pane, wire encoding |
| `crates/kampr-node` | axum server, `/ws`, herd model, `manage` ops |
| `crates/kampr-mesh` | Peer transport, node-to-node handshake, hub relay |
| `crates/kampr-auth` | Tiers, devices, tokens, passkeys, audit log |
| `crates/kampr-journal` | Claude and Codex transcript adapters |
| `crates/kampr-cli` | The `kampr` binary — `init`, `serve`, `setup`, `service` |
| `crates/kampr-spike` | End-to-end fidelity check against Herdr's own grid |
| `client/shared` | Theme tokens, WS client, herd model, navigation |
| `client/terminal` | Cell renderer, zoom and pan, key row, input capture |
| `client/conversation` | Markdown, tables, tool cards, diffs |
| `client/{android,desktop,web}App` | Composition roots |
| `packaging/` | Install script, plugin action dispatcher, systemd unit, launchd agent |
| `.github/workflows/release.yml` | Bundle → binary → checksum → signature → published release |
| `docs/01-implementation-findings.md` | What Herdr exposes and what is possible |
| `docs/02-roadmap.md` | Tickable plan |
| `docs/03-probe-log.md` | Every claim about Herdr, traced to a command |
| `docs/04-wire-protocol.md` | The node ↔ client contract |
| `docs/05-agent-briefs.md` | Parallel workstreams |
| `docs/06-audit.md` | Completeness audit — what is broken, incomplete, and missing |
| `ARCHITECTURE.md` | Why Kampr is shaped the way it is |
| `docs/adr/` | Decision records, each with what would justify revisiting it |
| `docs/07-android-release.md` | Signing, release and kobup publish |
| `docs/08-threat-model.md` | Assets, adversaries, and the residual risks |
| `docs/07-mesh-deployment.md` | One hub behind Nginx Proxy Manager, peers dialling out |
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
