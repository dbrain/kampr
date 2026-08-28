# One hostname, every host

The deployment this was built for: **Nginx Proxy Manager points at one Kampr node, and that node
serves the panes of every machine you own.** Nothing but the hub needs an address, a certificate,
or a hole in a firewall.

```
  phone / laptop browser
        │  https://kampr.example.com          (one hostname, one certificate)
        ▼
  Nginx Proxy Manager        ── TLS terminates here
        │  http://127.0.0.1:8790              (loopback, same host)
        ▼
  kampr node  "front"  ── hub role ──────────────┐
        │ herdr sessions on this host            │  /mesh, peers dial IN
        ▼                                        │
   panes on front                    ┌───────────┴───────────┐
                                     │                       │
                              kampr node "laptop"     kampr node "workshop"
                              (NAT, no inbound)       (NAT, no inbound)
                                     │                       │
                              panes on laptop         panes on workshop
```

The peers dial **outbound**. That is the whole trick: a laptop on a café network, a machine behind
CGNAT, a box on a friend's LAN — each opens a WebSocket *to* the hub and serves it from there. You
never forward a port to a laptop, and the proxy has exactly one upstream.

---

## 1. The hub

```bash
kampr init --bind 127.0.0.1:8790 --origin https://kampr.example.com
```

Two things matter here and both are easy to get wrong.

**`--bind 127.0.0.1`.** With a proxy in front, the node should not also be reachable directly. If it
is, an attacker who can reach it can forge the `X-Forwarded-For` header that the rate limiter keys
on. Bind loopback when the proxy is on the same host. If NPM runs in Docker, loopback inside the
container is not your host — bind the Docker bridge address (commonly `172.17.0.1:8790`) or the
host's LAN address, and firewall the port to the bridge.

**`--origin https://kampr.example.com`.** The node's same-origin allowlist is derived from this,
never from the request's own `Host` — reflecting `Host` would let a DNS-rebinding attacker satisfy
the check with their own header. Get it wrong and every browser WebSocket upgrade is refused with
`cross-origin request refused`. It is also the WebAuthn RP ID, and a passkey does not survive a
change of origin, so set it **before** enrolling one.

Then, in `config.toml`:

```toml
[server]
bind = "127.0.0.1:8790"
origin = "https://kampr.example.com"
# Opt-in, never inferred. Set this ONLY when the node is unreachable except through the proxy:
# X-Forwarded-For is trivially forgeable by anyone who can reach the node directly, and a forged
# one hands an attacker a fresh rate-limit bucket per guess.
trust_proxy = true

[mesh]
# The hub role, and the one line here the hub cannot skip. Off by default — enrolment decides who
# may join, this decides whether the door exists at all — so without it `kampr mesh invite`
# refuses and `POST /api/mesh/invite` answers 409.
accept = true
```

Those comments are yours, not the tool's: re-running `kampr init` rewrites `config.toml` through
the TOML serialiser and every comment in it goes. The *values* survive — `--force` carries the
identity, the bind, the origin, `trust_proxy`, `extra_origins` and `[mesh]` forward and prints what
it keeps — but the annotations do not.

Then `kampr service install`, which writes the user unit **and** turns on lingering for your user.
That second half is not optional: a `systemd --user` manager is torn down when your last session
ends and is not started at boot without it, so a unit installed on a non-lingering user dies at
logout and does not come back after a reboot. If the install cannot do it — no privilege, no
logind — it prints `loginctl enable-linger <you>` as a required next step, and `kampr doctor`
fails on the `linger` check until you run it.

Then pair your phone as usual.

---

## 2. Nginx Proxy Manager

New **Proxy Host**:

| Field | Value |
|---|---|
| Domain Names | `kampr.example.com` |
| Scheme | `http` |
| Forward Hostname / IP | `127.0.0.1` (or the bridge/LAN address — see above) |
| Forward Port | `8790` |
| **Websockets Support** | **ON** |
| Block Common Exploits | on |
| Cache Assets | **off** |

**SSL** tab: request a Let's Encrypt certificate, **Force SSL** on, **HTTP/2** on, HSTS optional
(the node sets it itself only when it terminates TLS, which here it does not).

### The two things people get wrong

**1. Websockets Support.** This toggle is what adds

```nginx
proxy_http_version 1.1;
proxy_set_header Upgrade $http_upgrade;
proxy_set_header Connection $http_connection;
```

Without it the upgrade never happens: `/ws` and `/mesh` answer with a plain HTTP response, the
browser reports a failed WebSocket, and the logs show nothing interesting because nothing went
wrong at the HTTP layer. If you hand-write the config instead of using the toggle, note that
`Connection` must be `upgrade` **for upgrade requests only** — hardcoding `Connection "upgrade"` on
every request breaks keep-alive for the plain HTTP endpoints.

**2. The idle timeout.** NPM's default `proxy_read_timeout` is 60 seconds. A watched pane that
produces no output for a minute — which is most panes, most of the time — has its socket cut, and
the client reconnects in a visible flicker. Add this in **Advanced → Custom Nginx Configuration**:

```nginx
location /ws {
    proxy_pass $forward_scheme://$server:$port;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $remote_addr;
    proxy_set_header X-Forwarded-Proto $scheme;
    proxy_read_timeout 3600s;
    proxy_send_timeout 3600s;
}

location /mesh {
    proxy_pass $forward_scheme://$server:$port;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $remote_addr;
    proxy_set_header X-Forwarded-Proto $scheme;
    proxy_read_timeout 3600s;
    proxy_send_timeout 3600s;
}
```

When the Proxy Host is saved, `kampr doctor` on the node will tell you whether any of this
actually worked: its `origin` check fetches `{origin}/api/node` and compares the node id it gets
back against this node's own. A port open at the hostname proves nothing — a wrong Forward Port, a
Docker-bridge address that is not your host, and NPM's own default page all leave 443 listening —
so that one request is the difference between "the DNS is right" and "the proxy leads here".

`/mesh` is the peer transport and needs exactly the same treatment as `/ws`. It is on the same
hostname on purpose: one proxy host, one certificate, one thing to get right.

A mesh link pings every five seconds, so it survives a 60 s timeout on its own — but a *client*
socket watching a quiet pane does not, and both deserve the same setting.

### Three things about that block that are not optional

**`X-Forwarded-For $remote_addr` is the line that keeps `trust_proxy` honest.** nginx passes a
client request header straight through when the config does not set it, and the node reads
`X-Forwarded-For` and nothing else — never `X-Real-IP`. Without that line, a request arriving from
the internet with its own `X-Forwarded-For: 203.0.113.7` is recorded and rate-limited as
`203.0.113.7`, which hands every attacker a fresh bucket per guess: exactly the forgery the
warning at the top of §1 is about, re-opened by the snippet meant to fix a timeout. The whole block
above was run against a real nginx replicating what NPM generates: with these two lines the node's
audit log records `peer 127.0.0.1`, without them it records `peer 203.0.113.7` — the value the
client chose — and either way the upgrade itself returns `101 Switching Protocols`.

Use `$remote_addr`, **not** `$proxy_add_x_forwarded_for`. Appending preserves the forged value and
leaves the node to guess which entry is real; replacing is what makes the header un-forgeable, and
replacing is what NPM's own `proxy.conf` does for `location /`. Matching it is the point.

**`proxy_pass` must name the same upstream the Proxy Host form does.** `$forward_scheme`,
`$server` and `$port` are set by NPM at server scope, so the block above follows the form
automatically. If you write a literal address instead, write the *same* one you put in the form —
a hardcoded `http://127.0.0.1:8790` inside an NPM Docker container is the container's own
loopback, not your host, so `/ws` and `/mesh` break while every other path keeps working. That
failure looks like "the page loads and the terminal never connects".

**A custom `location` bypasses everything attached to the Proxy Host.** Block Common Exploits, an
Access List, HTTP basic auth: those are configured on `location /`, and these two blocks are not
`location /`. The two endpoints that carry the terminal and the mesh are therefore the two with no
NPM-level protection in front of them — which is fine, because the node authenticates both itself,
but it is not what an access list on the Proxy Host looks like it is doing. If you rely on an
access list, repeat its `allow`/`deny` lines inside both blocks.

---

## 3. Join the other hosts

Each peer is a Kampr node in its own right, so start by giving it a config and an identity:

```bash
kampr init --name laptop
```

The defaults are correct for a peer — loopback bind, no origin, no certificate, and no `[mesh]`
section to write, because `accept = false` is already the default and a peer dials out rather than
accepting anything. `kampr mesh join` opens the same device database `init` creates, so without
this step it fails with *"run `kampr init` first"*.

Then, on the hub:

```console
$ kampr mesh invite
Join code for a node, valid 10 minutes, one node:

  kampr mesh join --hub https://kampr.example.com --code K7QF-9M2X \
      --fingerprint 3f8a-91cd-04b2-77e1

  hub          front (01JB2K…)
  fingerprint  3f8a-91cd-04b2-77e1
```

On each peer, run exactly that. The code is single use and short lived; the ed25519 key is the
credential from then on, so a peer reconnects unattended forever after. `--fingerprint` is checked
**before** the peer signs anything, which is what turns a first connection from trust-on-first-use
into a confirmed one — the hub's fingerprint is also printed by `kampr status` on the hub.

```console
$ kampr mesh join --hub https://kampr.example.com --code K7QF-9M2X --fingerprint 3f8a-91cd-04b2-77e1
dialling wss://kampr.example.com/mesh …

joined front (01JB2K…)
  url          wss://kampr.example.com/mesh
  fingerprint  3f8a-91cd-04b2-77e1

`kampr serve` keeps the link up from here, and reconnects on its own.
```

The peer needs **no** `origin`, no certificate, no open port, and no reachable address — it does
need the `kampr init` above, because the mesh key and the peer's device database live in the
directories `init` creates. It needs to be able to make an outbound HTTPS connection, which is the
same thing as being able to browse.

Check it from the hub:

```console
$ kampr mesh list
  peers
    laptop               91cd-3f8a-77e1-04b2 enrolled
    workshop             c40a-1188-9e33-2b5d enrolled
```

And in any client: one herd, with `front`, `laptop` and `workshop` in it, each with its own
`rtt_ms`, its own herdr version, and its own kampr build.

---

## 4. Cutting a node off

```bash
kampr mesh revoke laptop        # by name, node id, fingerprint or key
```

The live link drops within seconds — a revocation has to bite on the connection that is already
open, not at the next handshake. `kampr mesh forget` removes the row entirely.

A key, a fingerprint and a node id name exactly one node. A **name** does not have to: a peer picks
the name it enrols under, so two can answer to `laptop` — and rather than cut off whichever the
table reached first, `revoke` prints both and cuts off neither. Name one by its fingerprint when it
does.

From the other side, a peer can cut the hub off too: the hub holds a device row on each peer
(`mesh:…`, visible in that peer's device list), and revoking it refuses the hub even though its key
is still enrolled.

---

## 5. What this costs

Frames cross two hops, so a peer pane's echo latency is the local ~27 ms plus the WAN round trip to
the hub plus the round trip from the hub to you. A 40 ms link makes a peer pane feel like SSH to
that host, because that is what it is. The client is *told* the number — `rtt_ms` per node — so a
slow peer looks slow rather than silently lagging.

Bandwidth does not scale with viewers: the hub holds one `watch` per pane however many clients are
looking at it, and Herdr coalesces bursts to grid state, so a reconnect costs one full grid of
about 4 KB rather than a replay.

---

## 6. Tier 0 mesh, without a proxy

The mesh does not require any of the above. `kampr mesh join --hub http://192.168.1.24:8790` works
on a LAN, dials `ws://`, and is authenticated exactly the same way — the ed25519 handshake does not
care what carried it. What you lose is confidentiality: without TLS the link is in clear, so the
hub should be on a network you trust. The node's own persistent "unencrypted" banner already says
so for browsers, and the same reasoning applies here.

## 7. Threat notes

- **TLS terminates at the proxy.** The proxy → node hop is plaintext HTTP. That is fine when they
  share a host and loopback; it is not fine across a network. If the proxy is elsewhere, give the
  node its own certificate (`[server.tls]`) and point the proxy at `https://`.
- **Mesh authentication is not encryption.** The ed25519 handshake proves *who*, and TLS provides
  *secrecy*. On a link where you do not trust the transport, do not rely on the handshake to hide
  anything.
- **`trust_proxy` is a promise you are making**, not a fact the node can check. It says "the only
  way to reach me is through the proxy". If that stops being true, turn it off.
- **A join code is a credential for ten minutes.** It enrols exactly one node and dies. A leaked
  code that is still live enrols an attacker's node as a *peer*, which means the hub can watch its
  panes — not the other way round. Revoke it and re-invite.
