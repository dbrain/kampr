# 0007 — Peers dial outbound to a hub

- **Status:** Accepted
- **Date:** 2026-08-20
- **Implementation:** `crates/kampr-mesh`, `crates/kampr-auth/src/mesh.rs`, the node's accept and
  relay paths, and `kampr mesh invite|join|list|revoke|forget`. **Landed late and unproven** — unit
  coverage, no end-to-end test across two hosts. Check [`../06-audit.md`](../06-audit.md) for where
  it actually stands.
- **Evidence:** [findings §1.8](../01-implementation-findings.md), probe [#49](../03-probe-log.md)

## Context

"Many machines, one herd" is the goal Herdr contributes nothing to and blocks nothing about.
`herdr --remote` is not a federation API — it makes the *local* Herdr a thin SSH client of a remote
Herdr and streams a UI to a local terminal. There is no cross-host socket API, no remote JSON
endpoint. Multi-host is entirely Kampr's problem.

The shape falls out of two facts about where the machines actually are.

**A laptop is behind NAT and a desktop usually is too.** Any design where the phone — or a
coordinating node — dials each host directly needs an inbound path to each host: a port forward per
machine, a VPN, or a mesh overlay as a hard dependency. Every one of those is a prerequisite the
tier ladder exists to avoid ([ADR 0006](./0006-auth-is-in-the-node.md)).

**The operator has exactly one front door and does not want a second.** They already pointed a
hostname and a certificate at one machine, or they are content on a LAN IP. Asking for a second
reachable endpoint per host is asking for the setup to fail.

Both are answered by the same inversion: **make the reachable machine the only reachable machine, and
have everything else dial it.** Only the hub needs an inbound path. A laptop behind NAT joins by
connecting out, exactly as it connects out to anything else.

That leaves what runs over the link. The tempting answer is a second, mesh-specific protocol. The
better one was already sitting there: once a hub and a peer have authenticated each other, **the hub
is a client of the peer**, sending `watch` and `input` and receiving `grid.reset`, `grid.patch` and
`scrollback` — the ordinary v1 client protocol, backwards. The peer serves it with the very same
session code that serves a browser.

One consequence of Herdr's own shape makes the mesh worth more than it first appears: a Herdr TUI
client attaches to exactly one server, and **named sessions are separate servers** (#49). So even on
a single host, enumerating every named session gives a herd the TUI cannot show at once. Multi-host
and multi-session are the same feature.

## Decision

**Peers dial outbound to a hub. "Hub" is a role a node is configured into, not a separate build, and
the link carries the ordinary client protocol in reverse.**

- **The same binary dials out, accepts dial-ins, or both.** There is no hub build and no peer build.
- **A node's identity is an ed25519 keypair generated on first run, and the public key is the
  credential.** A node that regenerates its identity is a different node and must be enrolled again.
  Mutual: the peer holds the hub's key so it can refuse an impostor answering at that URL, and the
  hub holds the peer's key so it can recognise it when it dials.
- **Mesh auth is entirely separate from user auth.** Different table, different credential type,
  different enrolment flow. A compromised viewer session cannot impersonate a node, and a revoked
  device has no bearing on a link.
- **Joining uses a single-use, time-boxed, argon2id-digested invite code**, with the same
  attempt-burning rule as a device pairing code, and the key is pinned at first connection while the
  operator is looking at the fingerprint.
- **The relay is the v1 protocol, so the backpressure rule applies at both hops by construction.**
  There is no second congestion story to design, and a hub that falls behind costs a client one
  `grid.reset` out of the hub's own shadow rather than a round trip to the peer.
- **The hub keeps a decoded shadow of each remote pane** — grid, style table and history — so it can
  answer a new viewer immediately, and re-encodes per client so every client keeps only style ids it
  was told about.
- **Every node enumerates all named sessions on its host**, not just the default. Creating one is the
  single management action that shells out rather than calling a socket method, because Herdr's API
  has no method for it.

## Consequences

- **Only one machine needs to be reachable.** That is the whole dividend, and it is what makes the
  setup ladder survive contact with more than one host.
- **The hub is a single point of failure and a single point of trust.** If it is down, the herd is
  down; if it is compromised, every peer's terminals are compromised. This is a real centralisation
  and it is chosen over the alternative of demanding inbound reachability everywhere.
- **Frames cross two hops, so latency is 27 ms plus the WAN round trip.** That is still SSH-feel, and
  it is the honest cost of the shape.
- **A per-node latency indicator is not a nicety.** A 200 ms peer must look different from a local
  pane or the interface lies about what it is showing.
- **Global ids get a node component**, and every id a client sees is `<node_id>/<pane_id>`. That was
  designed into the protocol from the start rather than retrofitted — which is why the wire's node
  entry already distinguishes `local` from `peer` even while only `local` is ever produced.
- **A herd can now be partly stale.** Probe #70 already showed the local version of this failure: a
  herdr server was killed under a live watcher and nothing told the client — `online` stayed true for
  the whole outage. Across a mesh link that failure mode multiplies, and per-node health is the
  answer rather than a global connection state.
- **Splitting a view across instances needs no protocol support at all.** Each pane is an independent
  stream and nothing binds a view to one server, so a Kampr window showing four panes from three
  machines and two sessions is just four watches.

## What would justify revisiting

- **Direct peer-to-peer links between two reachable nodes.** Nothing in the design forbids them, and
  the handshake is symmetric enough to support them; the reason not to build them first is that the
  NAT'd case is the common one and a design that handles only the easy case handles nothing. If both
  ends are reachable, dialling directly is a strict improvement and the hub becomes an optional
  rendezvous rather than a required relay.
- **A relay that does not decode.** The hub currently decodes a peer's cell grid and re-encodes it
  per client, which costs CPU on the hub and is what allows the shadow. If hub CPU ever matters more
  than instant fan-out, a pass-through mode is possible — at the cost of the shadow, and therefore of
  the "new viewer sees the grid immediately" property.
- **Herdr growing a cross-host API.** Then most of this is redundant. It has been explicitly out of
  scope for Herdr, so it is not something to wait for.
