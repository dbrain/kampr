# Kampr

Remote access to a [Herdr](https://herdr.dev) agent herd from a phone, a tablet or a browser —
across every machine you own, behind real authentication, without reshaping the session you left
running at your desk.

> **Status: released, and young.** Every
> [release](https://github.com/dbrain/kampr/releases) carries static Linux and macOS binaries for
> both architectures, a checksum-verified installer, and a signed Android APK; `kampr update` takes
> the next one. What is still unproven is unproven for want of hardware rather than for want of a
> test — no passkey has ever been created on a real phone, push has been driven against a real
> browser and a real push service but never against a handset, and the mesh has never crossed a real
> WAN. [`docs/06-audit.md`](docs/06-audit.md) is the standing list. Read it before you expose this
> to anything.

- **A live terminal, not a text mirror.** Kampr streams Herdr's own rendered frames and
  reconstructs the grid exactly — truecolour, cursor, hyperlinks and all.
- **A conversation view for agents.** Claude, Codex and Antigravity transcripts render as real markdown, so a
  table is a table. This is the default view for agent panes.
- **Looking never reshapes.** Kampr observes and types; watching a pane cannot change it, however
  small the screen. Small screens are handled with zoom, pan and the conversation view, so the desk
  you come back to is the desk you left. There is one deliberate exception, behind a panel and a
  confirmation — a pane that was *born* unusable, which happens when an agent starts a herdr
  headlessly, can be given a real width. It is the only thing that claims a PTY, it hands it back
  immediately, and it refuses to make a pane smaller than 80x24.

---

## TL;DR

Herdr 0.8.2 or newer, running. Then, on the machine it is running on:

```bash
curl -fsSL https://github.com/dbrain/kampr/releases/latest/download/install.sh -o install.sh
sh install.sh                                  # ~/.local/bin/kampr, checksum verified
kampr init --name laptop --bind 0.0.0.0:8790   # config, identity, URL, pairing code, recovery code
kampr service install                          # systemd --user unit, or a launchd agent on macOS
kampr doctor                                   # one ok/warn/fail line per thing that has to be true
```

Open the URL `kampr init` printed, on the phone, and pair with the code it printed beside it. The web
client is compiled into the binary, so a browser is the whole client and there is nothing else to
install — on Android there is a signed APK on the same release if you would rather have an app. Write
the recovery code down on paper; it is shown once.

`--bind 0.0.0.0:8790` is what makes it reachable from the phone. Leave it off and the node is
loopback-only, which is the default because anything that pairs with it can type into every terminal
on that host. It is plain HTTP on your LAN at this point, and the node says so on every screen until
you give it a hostname and a certificate.

That is one machine. Several machines behind one hostname, a certificate, passkeys and notifications
are all below, and all of them are optional.

Two notes on the installer, since it is fetching a binary that will hold your shells. It writes the
script to a file rather than piping into `sh` on purpose — a piped `curl` sends nothing into `sh`
when the URL 404s and the pipeline still exits 0. And it checks the tarball twice where it can: the
`SHA256SUMS` always, and the [cosign](https://docs.sigstore.dev/cosign/system_config/installation/)
signature beside it when a cosign is on the host. The second check is the one that matters — the
checksums come from the same server as the tarball, so they say who *served* it and never who built
it — but cosign ships on no distribution by default, so a host without one is told the signature
went unchecked rather than being stopped. A cosign that is present and *rejects* the signature is
still fatal, and so is a checksum that does not match. `KAMPR_REQUIRE_SIGNATURE=1` refuses to
install without a verified signature, if you would rather have the old behaviour.

---

## Vibe coded

Kampr was written overwhelmingly by an AI agent, in about a week, and the commit log does not hide
it. The phrase usually means generated fast and trusted blindly, so here is what was actually done
instead, all of it in this repository and all of it checkable.

[`docs/03-probe-log.md`](docs/03-probe-log.md) is 267 numbered rows today. Each one is a single claim about
Herdr or about a browser or about Google, the command that produced it, and what came back. Nothing
here is allowed to be reasoned out from another terminal multiplexer or recalled from training; if a
fact is not in that log, the rule is to go and measure it and add a row. The `#123` citations
scattered through the code point into it, and the constants that look arbitrary are mostly one of
those measurements. A fix is not accepted until the test that catches it has been run against the
*old* code and seen to fail, because a test that still passes with the defect restored is a harness
that was never the app. The integration suite drives a real Herdr binary end to end and skips loudly
rather than passing quietly when one is missing. Decisions that closed off other options are written
down in [`docs/adr/`](docs/adr/), each with what would justify revisiting it.

If the real question is whether to give this a shell on your machine, the two documents that answer
it are [`docs/08-threat-model.md`](docs/08-threat-model.md) — which is candid about what each rung
of the ladder does *not* protect against — and the probe log.

---

## The shape: one door, many rooms

There are **two roles, one binary.** Every machine that runs Herdr runs its own `kampr` node
alongside it — a node always serves the Herdr sessions on its own host, and nothing else. What
differs between them is only this: **does anything dial in, or does it dial out?**

- **The hub** — *the server you connect to.* One node, on the machine that is always on. It has the
  hostname, the certificate and the reverse proxy. Your phone talks only ever to this.
- **The peers** — *an instance on each box running Herdr.* Your laptop, the workshop machine, the
  one at your parents' house. Each dials **outbound** to the hub and serves its panes back down
  that connection.

```
  phone / laptop browser
        │  https://kampr.example.com          one hostname, one certificate
        ▼
  Nginx Proxy Manager        ── TLS terminates here
        │  http://127.0.0.1:8790
        ▼
  ┌─────────────────────┐
  │  kampr node "front" │  ← the hub. Serves front's own Herdr panes,
  │  + herdr on front   │    and relays every peer's.
  └─────────┬───────────┘
            │  /mesh — peers dial IN, over the same hostname
      ┌─────┴──────┬──────────────┐
      ▼            ▼              ▼
  kampr+herdr  kampr+herdr   kampr+herdr
   "laptop"     "workshop"     "parents"
   (NAT)        (NAT)          (NAT)
```

Peers dialling out is the whole trick. A laptop on a café network, a box behind CGNAT, a machine on
someone else's LAN — each opens a WebSocket *to* the hub. **You never forward a port to a peer, no
peer needs a certificate or a DNS name, and your proxy has exactly one upstream.** A peer needs only
to be able to make an outbound HTTPS connection, which is the same thing as being able to browse.

If you only have one machine, it is the hub and you are done after step 3 — the mesh is entirely
optional.

---

## Install

### 0. What you need

| | |
|---|---|
| Herdr | **0.8.2 or newer** (protocol 20), running, on every box |
| OS | Linux or macOS 11+. **Windows is not supported** — the node reaches Herdr over a Unix socket and supervises itself with systemd or launchd. Use WSL2. |
| To build *from source* | Rust 1.90+ and **JDK 21 exactly** — every module pins `jvmToolchain(21)`, so a newer JDK is not a substitute. Not needed if you install a release. Full list in [`docs/09-toolchain.md`](docs/09-toolchain.md) |

Kampr access is **unrestricted command execution** on the host it runs on. Treat a paired device
the way you would treat an SSH key.

### 1. Install the binary — on every box

```bash
curl -fsSL https://github.com/dbrain/kampr/releases/latest/download/install.sh -o install.sh
sh install.sh
```

`KAMPR_PREFIX` chooses where it lands; the default is `~/.local/bin`. It picks the tarball for this
OS and architecture, checks it against the release's `SHA256SUMS`, and checks that against the
cosign bundle beside it, pinned to this repository's release workflow. It refuses rather than
half-succeeding: a checksum mismatch installs nothing, a release with no `SHA256SUMS` installs
nothing, a release with no signature installs nothing, a signature that does not verify installs
nothing, and a new binary that will not run on the host is put back (#156). A host with no `cosign`
installs and is told the signature was not checked; `KAMPR_REQUIRE_SIGNATURE=1` makes that fatal
too.

Linux binaries are statically linked against musl, so there is no glibc floor and an old Raspberry Pi
is fine. macOS needs 11.0 or newer.

**From source instead.** The node **embeds** the web client, so the bundle is staged before cargo
runs, and the order is the part that is easy to get wrong:

```bash
git clone https://github.com/dbrain/kampr && cd kampr

# Stage the web client into the node's embedded assets.
# `env -u GRADLE_HOME` matters: a system GRADLE_HOME overrides the wrapper and
# builds with the wrong Gradle. The Makefile does the same thing for the same reason.
cd client && env -u GRADLE_HOME ./gradlew :webApp:stageNodeBundle && cd ..

# KAMPR_REQUIRE_BUNDLE=1 fails the build rather than quietly producing a binary
# that serves a placeholder page instead of the client.
KAMPR_REQUIRE_BUNDLE=1 cargo build --release -p kampr-cli

install -Dm755 target/release/kampr ~/.local/bin/kampr
```

### 2. Give every node an identity

Run `init` once per box. On a **peer** (any box that is not the one you connect to):

```bash
kampr init --name laptop     # --name is what you will see in the herd
```

On the **hub** — the box with the hostname — give it its address now:

```bash
kampr init --name front --bind 127.0.0.1:8790 --origin https://kampr.example.com
```

Then, on every box, install the service:

```bash
kampr service install    # systemd --user unit, or a launchd agent on macOS
```

`install` also writes down **which `herdr` it resolved**. A node reaches Herdr two ways — the
socket, for the herd and for input, and a spawned `herdr terminal session observe`, which is the
entire grid stream — and only the socket is pinned by the unit. A `systemd --user` manager's `PATH`
is `/usr/local/bin:/usr/bin:/bin` with no `~/.local/bin` in it, so a node that resolved the binary
through `PATH` served a correct herd, accepted input, and showed a blank grid in every pane, for
ever. So `init` and `service install` both record the absolute path in `config.toml`, a bare
`binary = "herdr"` also looks in the directory kampr itself was installed to — put both binaries in
the same prefix and nothing has to be configured — and `kampr doctor` runs the binary the observer
will run, the way it will run it.

This also enables **linger** for your user, which is what lets a `systemd --user` unit keep running
after you log out and start again at boot. Without it a node simply disappears at the next reboot,
so if `kampr service install` cannot enable it — it needs a privileged bus — it prints the exact
command as *required*, not as a suggestion. `kampr doctor` fails on an installed unit whose user
does not linger, so you will not find out by surprise.

Changing `--bind` or `--origin` later is an ordinary edit — re-run `init` with the new flags and it
rewrites them in place. `--force` resets the tuning sections but keeps your identity, bind, origin,
`trust_proxy` and `[mesh]`, and prints what it keeps and what it resets before writing. Only
`--new-identity` regenerates the node id, and it tells you that doing so stops every enrolled
passkey and every mesh peer pinned to it from working.

`init` writes `~/.config/kampr/config.toml`, generates this node's ed25519 identity, and — because
it is also the first-run pairing step — prints a URL, a QR code, a **pairing code** and a
**recovery code**.

```
Kampr node laptop (01M0GMAMNNCW7P6Y7WFK3WZWMD)
  config      /home/you/.config/kampr/config.toml
  state       /home/you/.local/state/kampr
  herdr       /home/you/.local/bin/herdr — recorded in config.toml, because a service manager's PATH is not your shell's
  identity    fdb5-34de-58b3-129e
  push key    BO81QvDTOqOLDJoR1q3oaxczsUz5BU0dEmARQ0bXCn0dsoyoi3iAAvqoHabwjBt36Ck8clGFmQa4YgcVHvrwyDc

  http://127.0.0.1:8790
  bind        127.0.0.1:8790 — this machine only. A phone on the LAN cannot reach it.
                           `kampr init --bind 0.0.0.0:8790` opens it to the network.

  [a QR code for that URL]

  pairing code   J5XD-V3ZH
  valid for      10 minutes, one device

  tier 0   secure context
  passkeys no   notifications yes   install yes

  RECOVERY CODE   3DXP-LMSQ-KVDP-VZ8Y-RW5E
```

**Write the recovery code down, on paper.** It is shown once and never again — the node keeps only
a slow one-way digest, so nothing and nobody can read it back. If you lose every paired device it is
the only way back in; lose it too and the way in is a shell on that machine, or nothing.

On a **peer** you can ignore the pairing code entirely — you will reach its panes through the hub.
A peer needs no mesh configuration at all: `[mesh] accept` is **off by default**, so nothing can
dial in to a peer, which is exactly what you want on a box that only ever dials out.

### 3. Set up the hub

The two flags you gave the hub in step 2 are the two that are easiest to get wrong, so here is what
they do:

**`--bind 127.0.0.1`** — with a proxy in front, the node must not *also* be reachable directly. If
it is, anyone who can reach it can forge the `X-Forwarded-For` header the rate limiter keys on. If
Nginx Proxy Manager runs in Docker, loopback inside the container is not your host: bind the Docker
bridge address (commonly `172.17.0.1:8790`) and firewall the port to the bridge.

**`--origin`** — the same-origin allowlist is derived from this and never from the request's own
`Host`, because reflecting `Host` would let a DNS-rebinding attacker satisfy the check with their
own header. Get it wrong and every browser WebSocket upgrade is refused with
`cross-origin request refused`. It is **also the WebAuthn RP ID**, and a passkey does not survive a
change of origin — so set it before you enrol one.

Then in `~/.config/kampr/config.toml` on the hub:

```toml
[server]
bind = "127.0.0.1:8790"
origin = "https://kampr.example.com"
# Opt-in, never inferred. Set this ONLY when the node is unreachable except through the proxy.
trust_proxy = true

[mesh]
accept = true    # REQUIRED on the hub. Off by default, and `kampr mesh invite` refuses without it.
```

Start it, and check:

```bash
systemctl --user restart kampr
kampr doctor
```

### 4. Point Nginx Proxy Manager at it

New **Proxy Host**: domain `kampr.example.com`, scheme `http`, forward to `127.0.0.1` port `8790`,
**Websockets Support ON**, Cache Assets **off**. On the **SSL** tab request a Let's Encrypt
certificate and turn **Force SSL** on.

Two things bite people here:

1. **Websockets Support** is what adds the `Upgrade`/`Connection` headers. Without it `/ws` and
   `/mesh` answer with a plain HTTP response, the browser reports a failed WebSocket, and the logs
   show nothing interesting because nothing went wrong at the HTTP layer.
2. **The idle timeout.** NPM defaults `proxy_read_timeout` to 60 seconds. A watched pane that
   produces no output for a minute — which is most panes, most of the time — has its socket cut,
   and you see a reconnect flicker. Fix it in **Advanced → Custom Nginx Configuration** with a
   `location /ws` and a `location /mesh` block raising `proxy_read_timeout`/`proxy_send_timeout` to
   `3600s`.

   **A custom `location` block replaces NPM's own headers rather than adding to them, so it must
   re-set `X-Forwarded-For` itself:**

   ```nginx
   proxy_set_header X-Forwarded-For $remote_addr;    # NOT $proxy_add_x_forwarded_for
   proxy_set_header X-Forwarded-Proto $scheme;
   ```

   Omit that line and a forged `X-Forwarded-For` from any client on the internet reaches the node
   verbatim, handing the attacker a fresh rate-limit bucket per request on the two endpoints that
   carry your terminal. `$remote_addr` **replaces** the header where `$proxy_add_x_forwarded_for`
   would append to it, and replacing is what makes it un-forgeable. Note also that a custom
   `location` bypasses NPM's Block-Common-Exploits include and any access list you set on the Proxy
   Host, for exactly those two endpoints. The full block is in
   [`docs/07-mesh-deployment.md`](docs/07-mesh-deployment.md).

### 5. Join the peers to the hub

On the **hub**:

```console
$ kampr mesh invite
Join code for a node, valid 10 minutes, one node:

  kampr mesh join --hub https://kampr.example.com --code B7RE-2WDN \
      --fingerprint fdb5-34de-58b3-129e
```

Run exactly that line on the peer. The code is single-use and short-lived; the ed25519 key is the
credential from then on, so the peer reconnects unattended forever after. `--fingerprint` is checked
**before** the peer signs anything, which is what turns a first connection from trust-on-first-use
into a confirmed one.

Check from the hub with `kampr mesh list`, and cut one off with `kampr mesh revoke laptop` — which
drops the live link within seconds rather than waiting for the next handshake.

### 6. Pair your phone

Open the hub's URL, and pair with a code from `kampr setup` (interactive) or `kampr pair`. Use
`kampr pair --readonly` for a device that should watch but never type.

```console
$ kampr setup
  1  pair a device            2  pair a read-only device
  3  list devices             4  revoke a device
  5  install the service      6  refresh
  7  new recovery code
```

Once the hub has a hostname and a certificate, **passkeys, notifications and add-to-home-screen
unlock** — none of which are possible against a bare IP, because a WebAuthn RP ID must be a
registrable domain and HTTPS on an IP address is not enough. `kampr status` tells you which tier you
are on and what is still locked.

A passkey in the **Android app** needs two things a browser does not, and a published APK cannot
carry one of them. The app has to name your hostname in its own manifest, compiled in when the APK
was built — so a build handed to strangers can never have it, and Kampr hides its passkey control on
a build that names nothing (#288). And Google's servers have to be able to fetch
`/.well-known/assetlinks.json` from your hostname over the public internet, which a hostname
resolving publicly to a private address does not satisfy however correct everything else is (#170).
**A passkey in the browser needs neither**, which is the route to use.
[`docs/10-passkeys.md`](docs/10-passkeys.md) is the whole story, including what you keep if you skip
it — which is a supported configuration and what most nodes run as.

---

## Back this up

Three files are irreplaceable, and none of them is in this repo:

| Path | What dies without it |
|---|---|
| `~/.local/state/kampr/kampr.db` | every paired device, token and passkey; every mesh enrolment; push subscriptions; per-pane preferences |
| `~/.local/state/kampr/vapid.pem` | **every push subscription — even if you restore the database.** A browser stores the VAPID public key *inside* the subscription it made, so a new key does not match an old subscription |
| `~/.config/kampr/node.key` | this node's mesh identity. Peers pin the fingerprint at join, so a new key means re-joining every one of them |

`config.toml` is worth keeping too, but it is a few lines you could retype. The three above are not.

```bash
systemctl --user stop kampr
tar czf kampr-backup-$(date +%F).tar.gz     -C ~ .config/kampr .local/state/kampr
systemctl --user start kampr
```

Stop the node first: SQLite's WAL means a live copy can be a torn one. Restore by unpacking to the
same paths on a node that is stopped, then starting it — the file modes matter (`0700` on both
directories, `0600` on the database, its two sidecars, `node.key` and `vapid.pem`), and `tar` will
preserve them if you restore as the same user. `kampr doctor` checks them and says so if they slip.

The Android release keystore is a separate irreplaceable thing with its own section in
[`docs/07-android-release.md`](docs/07-android-release.md) — losing it orphans every device that has
installed the app.

---

## Day to day

| | |
|---|---|
| `kampr status` | What this node is, whether it is reachable, which tier, how many devices |
| `kampr doctor` | One `ok`/`warn`/`fail` line per thing that has to be true, each with the command that fixes it; `--json` for scripts |
| `kampr setup` | The interactive ladder — pair, list, revoke, install the service |
| `kampr pair [--readonly]` | A pairing code, without the menu |
| `kampr recover` | Get back in when no paired device is left |
| `kampr mesh invite / join / list / revoke / forget` | Join hosts into one herd |
| `kampr service install / uninstall / status` | The user service that keeps the node running |
| `kampr` | **No subcommand: open this machine's herd in the terminal.** Exactly as bare `herdr` opens a session |
| `kampr connect <url> --code <code>` | Save a herd on another machine, so a bare `kampr` opens that one |
| `kampr url` | Print the node's URL |
| `kampr update [--check] [--version vX.Y.Z]` | Replace this binary with the latest release, verifying it first |

Installed as a Herdr plugin, the same actions appear in Herdr's own workspace menu (`herdr-plugin.toml`).

### The terminal client

`kampr` with no arguments opens the herd in the terminal you typed it in — herdr's layout and
herdr's keymap, over the mesh, across every node at once. It is the thing a herdr at the desk
structurally cannot be: a herdr TUI attaches to exactly one server, and this one shows panes from
several servers on several hosts in one window, with `⚑ blocked` one keystroke from the question an
agent is waiting on.

It finds a herd by itself, in this order: a node on this machine (it mints itself a device, named
`cli@<hostname>`, listed by `kampr setup` and revocable like any other), then a herd saved by
`kampr connect`, then it says how to pair and exits. It never prompts.

The keymap is herdr's — `ctrl+b` prefix, `ctrl+b ctrl+b` for a literal, `ctrl+b [` for copy mode —
and it is the **client's**, not the node's, for the reason herdr gives for `--remote`: local muscle
memory beats remote config. `KAMPR_TUI_PREFIX=ctrl+a` moves it out of the way so `ctrl+b` reaches
the pane's own program.

An agent pane opens on its **conversation**, not its grid, with the markdown rendered as markdown
and images inline where the terminal can draw them. `ctrl+b shift+V` switches to the live grid.

A pane wider than your terminal is cropped and panned rather than reflowed, because Kampr never
resizes a pane ([ADR 0002](./docs/adr/0002-kampr-never-resizes-a-pane.md)). It will ask *your
terminal* to grow instead, where your terminal supports that — ghostty and kitty do not, konsole
does ([#291](./docs/03-probe-log.md)) — and the status line always says which rung of that ladder
it used and why.

### Staying current

Each node asks GitHub once a day what the latest release is, caches the answer in its state
directory, and puts it beside its own build in the herd model. The phone's **Machines** list is
therefore the answer to the only version question a mesh actually raises — *which of my machines
are stale* — without logging into any of them:

```
front — this machine · kampr 0.1.21
back  — peer · kampr 0.1.19 · 0.1.21 available
```

A node with no route out says nothing rather than reporting an error, and the check is the
node's own: a hub never judges a peer's version, because only the peer knows what it is running
and only the peer's config can say whether it may ask at all. To turn it off entirely:

```toml
[update]
check = false
```

Taking an update is always a decision, never an event. `kampr update` replaces the binary this
command is running from, using the same download, the same SHA-256 check and the same cosign
signature check as `install.sh` — it *is* `install.sh`, embedded in the binary — and restarts the
service only when the installed unit names the binary it just replaced. It refuses rather than
half-succeeding: a checksum mismatch installs nothing, a release with no `SHA256SUMS` installs
nothing, a release whose signature cannot be checked installs nothing, `KAMPR_ALLOW_UNVERIFIED` and
`KAMPR_BASE_URL` in your shell are not inherited — by `kampr update` or by `herdr plugin install`,
which is the same installer — and a new binary that will not run on the host is put back.

```bash
kampr update --check               # say what is available, install nothing
kampr update                       # take it
kampr update --version v0.1.19     # go back to one that worked
```

**Nothing updates itself, and a hub cannot update a peer.** A process that can type into every
terminal on a host does not get to replace its own binary unasked, and a hub that could push
binaries to peers would turn one compromised machine into code execution on all of them.

### Check the pipeline end to end

```bash
herdr --session probe                          # a throwaway session in one terminal
HERDR_SESSION=probe cargo run -p kampr-spike   # in another
```

Reconstructs the pane's grid from the frame stream alone and diffs it against Herdr's own
`pane.read visible`. It should print `PERFECT MATCH`.

---

## Repo layout

| Path | |
|---|---|
| `crates/kampr-herdr` | Herdr socket client, snapshot model, `observe` supervisor, scrollback |
| `crates/kampr-term` | VT emulation → cell grid, dirty rows, OSC 8 |
| `crates/kampr-core` | Provider seam, pane registry, one emulator per pane, wire encoding |
| `crates/kampr-node` | axum server, `/ws`, herd model, `manage` ops |
| `crates/kampr-mesh` | Peer transport, node-to-node handshake, hub relay |
| `crates/kampr-auth` | Tiers, devices, tokens, passkeys, audit log |
| `crates/kampr-journal` | Claude, Codex and Antigravity (`agy`) transcript adapters |
| `crates/kampr-push` | VAPID, subscriptions, batching |
| `crates/kampr-cli` | The `kampr` binary |
| `crates/kampr-spike` | End-to-end fidelity check against Herdr's own grid |
| `client/shared` | Theme tokens, WS client, herd model, navigation |
| `client/terminal` | Cell renderer, zoom and pan, key row, input capture |
| `client/conversation` | Markdown, tables, tool cards, diffs |
| `client/mosaic` | Panes from several machines in one window |
| `client/{android,desktop,web}App` | Composition roots |
| `packaging/` | Install script, plugin dispatcher, systemd unit, launchd agent |

## The reasoning

| | |
|---|---|
| [`ARCHITECTURE.md`](ARCHITECTURE.md) | Why Kampr is shaped the way it is |
| [`docs/adr/`](docs/adr/) | Decision records, each with what would justify revisiting it |
| [`docs/01-implementation-findings.md`](docs/01-implementation-findings.md) | What Herdr exposes, and what is therefore possible |
| [`docs/03-probe-log.md`](docs/03-probe-log.md) | Every claim about Herdr, traced to the command that proved it |
| [`docs/04-wire-protocol.md`](docs/04-wire-protocol.md) | The node ↔ client contract |
| [`docs/06-audit.md`](docs/06-audit.md) | **What is broken, incomplete and missing** |
| [`docs/07-mesh-deployment.md`](docs/07-mesh-deployment.md) | One hostname, every host — the full proxy and mesh guide |
| [`docs/09-toolchain.md`](docs/09-toolchain.md) | Every version this builds with, and which ones bite |
| [`docs/07-android-release.md`](docs/07-android-release.md) | Signing, release and kobup publish |
| [`docs/08-notifications.md`](docs/08-notifications.md) | Push, from `agent_status` to a phone |
| [`docs/08-threat-model.md`](docs/08-threat-model.md) | Assets, adversaries and the residual risks |
| [`docs/10-passkeys.md`](docs/10-passkeys.md) | What passkeys need, what Android needs on top, and what you keep without them |
| [`docs/02-roadmap.md`](docs/02-roadmap.md) | The plan, and where each phase actually stands |

## Licence

MIT.
