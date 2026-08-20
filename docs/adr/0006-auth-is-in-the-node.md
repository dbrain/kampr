# 0006 — Auth is in the node, and the origin dictates the ladder

- **Status:** Accepted
- **Date:** 2026-08-20
- **Shipped in:** `836f1a0` (tiers, devices, tokens, passkeys), `413083a` (the security pass)
- **Argues against:** Collie [ADR 0001](https://github.com/AltanS/collie), *"Collie manages exactly one
  front door"*

## Context

Kampr's socket reach is unrestricted code execution on every host it can see. Whatever authenticates
a request is the only thing between a stranger and a terminal running as you, so it is a security
question before it is a convenience one.

Collie answers it by delegating: `tailscale serve` terminates TLS and injects
`Tailscale-User-Login`, trusted only from loopback; or a reverse proxy asserts a device header.
Collie is explicit that device ids "are names your proxy asserts, not secrets — treat them as
guessable". **There is no login, no credential, no session.** `tailscale funnel` is forbidden in
writing.

That is coherent, and it is coherent *because of a precondition*: the bridge binds loopback and the
tailnet is the boundary. Delete the precondition — "make it reachable from a hostname you control",
which is Kampr's Tier 1 and the recommended rung — and the model evaporates. There is nothing left
holding the door.

So the question is not "is Collie wrong" but "does Kampr have Collie's precondition". It does not,
deliberately: the whole point of the tier ladder is that Tailscale is one option and never a
dependency.

Then a second, unrelated constraint decides the *shape* of what replaces it, and it is a web
platform rule rather than a design preference:

> **A WebAuthn RP ID must be a registrable domain. An IP address is not one.** The working group
> considered allowing it and [declined](https://github.com/w3c/webauthn/issues/1358). `localhost` is
> the only non-domain exception.

HTTPS does not help. A self-signed certificate on `https://192.168.1.24:8790` buys a secure context
after the interstitial — so service workers may work — and still **no passkeys**, because the rule is
about the name, not the transport.

A third rule stacks on top: service workers, PWA install and the Push API all require a **secure
context**, which plain HTTP on a LAN IP is not. And iOS Web Push additionally works only for Home
Screen web apps.

Those rules do not produce a preference between deployments. They produce a **ladder**, where each
rung is defined by what the origin makes possible:

| Rung | Origin | Passkeys | Push / install |
| --- | --- | --- | --- |
| **Just run it** | `http://192.168.1.24:8790` | ✗ — not a registrable domain | ✗ — not a secure context |
| **Hostname + certificate** | `https://kampr.home.example.com` | ✓ | ✓ |
| Public, or Tailscale | same, differently reachable | ✓ | ✓ |

The middle rung is the one to document as the default. A reverse proxy with a DNS-01 wildcard gives
a real certificate on a LAN-only hostname — no port forwarding, no exposure, no Tailscale.

## Decision

**The node authenticates in process. A front door supplies TLS and nothing else.**

- **Device-bound sessions at every rung.** Enrolment mints a long-lived, revocable, per-device
  token; the device list is visible and each entry killable. A `readonly` role exists for devices
  you half-trust.
- **The bottom rung's auth is real for what it is**: a pairing code redeemed for a device token, a
  LAN bind that must be asked for, a persistent unencrypted banner, and a **30-day expiry that forces
  a deliberate decision to keep going**. Above it, a passkey is phishing-resistant and revocable, so
  its tokens do not expire.
- **Header trust is opt-in and is never the credential.** `trust_proxy` governs whether
  `X-Forwarded-*` is believed, and it reads the **hop the proxy appended** — the right-most entry —
  because reading the client-supplied head is what made the rate limiter forgeable by the traffic it
  exists to limit.
- **The tier is derived from the configured origin, never from the request.** A request cannot tell
  the node what it is. The same rule governs the same-origin allowlist, which is built from the bind
  address rather than reflected from the request's own `Host` — otherwise it satisfies itself under
  DNS rebinding.
- **What cannot work is absent, not offered and failing.** `hello.security` tells a client what this
  origin supports and `unlocks` names what a hostname would buy. A client builds its setup ladder
  from that message and never by parsing the URL. On an IP the passkey affordance does not exist.
- **Node-to-node auth is separate from user auth** ([ADR 0007](./0007-peers-dial-outbound-to-a-hub.md)),
  so a compromised viewer session cannot impersonate a node.

## Consequences

- **Kampr owns credential storage, rate limiting, an audit log and a revocation path** — all of it
  the kind of code that is wrong in ways nobody notices. Nine defects in this area were found by an
  audit after it shipped and fixed in `413083a`; the current standing list is in
  [`../06-audit.md`](../06-audit.md) and the honest account is in
  [`../08-threat-model.md`](../08-threat-model.md).
- **A pairing code alone stopped being a credential**, because printing one into a Herdr pane means
  every read-only device sees it. A code minted at the console is inert until an operator arms it
  from that console, for a 60-second window; the keypress is the channel a watching device does not
  have. The residual race — an attacker who read the code redeeming inside that window — is real,
  bounded by an attempt counter rather than closed, and is written up in the threat model.
- **Loopback is the default bind and `--bind` is an explicit act that prints what it means.** The
  first version of this shipped bound to `0.0.0.0` in cleartext, which reversed the project's own
  stated policy and crossed its own gate.
- **`security.tier` only ever takes the values 0 and 1**, because the only thing the node can detect
  from an origin is whether passkeys are possible. "Public" and "Tailscale" are deployment postures,
  not detectable states — a Tailscale hostname is a hostname. The four-tier taxonomy in
  [`../01-implementation-findings.md`](../01-implementation-findings.md) §3.7 is a documentation
  ladder for humans; the wire's `tier` field is a boolean, and
  [`../04-wire-protocol.md`](../04-wire-protocol.md) still describes it as four-valued.
- **A passkey does not survive a change of origin.** Credentials are scoped to the RP ID and a
  changed one makes every enrolled passkey inert without deleting it. The CLI says so where the flag
  is set; there is no migration path and no startup warning, which is a gap.
- **`localhost` is tier 1 and `127.0.0.1` is tier 0**, because the browser rule is about
  registrable domains and `localhost` is the single exception. Surprising, correct, and worth
  knowing before debugging it.
- **Kampr does not manage a front door and never will.** No `tailscale serve` sidecar, no proxy
  supervision. Collie's ADR 0001 reasoning — "we manage only what we run and can test" — applies with
  more force here, because Kampr's ladder deliberately spans several front doors and could not test
  any of them.

## What would justify revisiting

- **A deployment where the front door genuinely is the boundary and Kampr is behind it anyway.** That
  is Collie's model, and running Kampr on loopback behind `tailscale serve` is a legitimate way to
  operate it. It does not justify *removing* the node's own auth, because the auth is what makes the
  other rungs safe — but it does justify a documented "the tailnet is the boundary" posture that
  stops asking for a second credential.
- **The WebAuthn working group reversing on IP addresses.** The ladder collapses to one rung and most
  of this reasoning goes with it. It is worth checking rather than assuming; the decision has been
  stable since 2019.
- **Anything that makes the bottom rung materially safer.** The 39.6-bit pairing code, the arming
  race and the bearer-token model are all consequences of "no domain, therefore no passkey". A
  credential that works on an IP and is not a shared secret would replace all three.
