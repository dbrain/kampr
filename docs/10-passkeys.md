# Passkeys, and the public-hostname problem

Written for someone who has read that Kampr supports passkeys, started to set them up, hit Digital
Asset Links, and wants to know whether it is worth continuing. The short answer is that the Android
half asks for something most people running Kampr do not want to give — a hostname that Google's
servers can fetch a file from — and that the constraint is structural rather than a configuration
you have not found yet. So the constraint comes first here, and the steps come after it.

This page is about *reachability and enrolment*. What a passkey defends against and what it does not
is [`08-threat-model.md`](./08-threat-model.md) §4 and §7, and that is the file to read if the
question is security rather than setup.

## What you are using instead

Nothing on this page is a prerequisite. A node with no passkey anywhere is a supported
configuration, not a degraded one, and it is what every Kampr node runs as until someone changes it.

The baseline credential is a **pairing code exchanged for a bearer token**. `kampr pair` prints a
code that is valid for ten minutes and for one device; the device posts it once and gets a token
back; the token is what every later connection presents. Neither is stored. The code is held as an
Argon2id digest, because ~39.6 bits read off a screen needs the work factor; the token is 256 bits of
system entropy and is held as a plain SHA-256, because nothing is going to search that space and
stretching it would only cost a hash on every request. So a copy of `kampr.db` yields no credential
that works. Attempts are rate-limited and land in the audit log, and a device is revocable from the
device list or from `kampr setup` — or never given write access at all, with `kampr pair --readonly`.

The one thing that is materially different: **a token is bearer** — anything that can read it can
use it. On a Tier 0 node, which is plain HTTP, "anything that can read it" includes anything on the
same network, and that is the actual cost of staying here. It is the same cost the unencrypted banner
is warning about on every screen.

Tier 0 tokens also **expire after 30 days** (`[auth] token_days`), and a device re-pairs when they
do. Note that this is a property of the *tier* and not of passkeys: `Auth::expiry` returns `None` —
no expiry at all — as soon as the origin can carry a passkey, whether or not anyone has enrolled one.
So a hostname with a certificate ends the 30-day cycle on its own.

## What a passkey would change

Two things, and only two.

- **It is bound to an origin**, so it cannot be phished. A page that looks like your node but is not
  at your node's origin cannot get the browser to sign anything with it.
- **It is not bearer.** There is nothing to copy off the wire or out of a backup; the private half
  never leaves the authenticator, and unlocking it is a gesture on the device rather than a secret
  that travels.

The never-expiring token is not on this list on purpose. It is real, and it is bought by the
hostname and the certificate rather than by the passkey — see above.

It does *not* buy you a new device. Registering a passkey attaches a credential to the device row
that is already paired and asking — it does not mint a second one. That is deliberate and it was a
defect once: enrolment used to create a fresh `full` device with its own never-expiring token, so
revoking the phone you could see in the list left a second, invisible one alive. Revoking the device
now kills the credential, both tokens, and the authentication start with it.

## What the browser needs

Two rules, neither of them Kampr's, and neither negotiable.

1. **A WebAuthn RP ID must be a registrable domain.** An IP address is not one, and the working
   group [declined](https://github.com/w3c/webauthn/issues/1358) to make it one. HTTPS does not
   change this: `https://192.168.1.24:8790` is a secure context and still cannot hold a passkey.
2. **A secure context**, which means HTTPS, with `localhost` as the single carve-out.

`Tier::detect` in `crates/kampr-auth/src/tier.rs` applies exactly those two rules and nothing else,
and the answer is what `hello.security` carries to every client:

| Origin | `secure_context` | `passkeys` | `tier` |
|---|---|---|---|
| `http://192.168.1.24:8790` | no | no | 0 |
| `https://192.168.1.24:8790` | yes | no | 0 |
| `http://127.0.0.1:8790` | yes | no | 0 |
| `http://localhost:8790` | yes | yes | 1 |
| `http://kampr.example.com:8790` | no | no | 0 |
| `https://kampr.example.com` | yes | yes | 1 |

**There are two tiers, 0 and 1, and the only thing that separates them is whether a passkey is
possible.** `tier` is literally `u8::from(passkeys)`. Push and add-to-home-screen ride on
`secure_context` instead, which is why HTTPS on a bare IP unlocks those two and not the third.

A hostname with a certificate is all the browser asks for, and it does not care whether that hostname
resolves publicly. A DNS-01 wildcard on a name that never leaves your LAN satisfies both rules — that
is the recommendation in the threat model's §8, and it costs nothing exposed.

**Not measured.** No passkey has ever been created against a Kampr node, in a browser or anywhere
else. A stock Android emulator with no Google account has no credential provider at all and answers
`No create options available` (#116), so the ceremony has been verified up to the provider and no
further. Everything in this section is the specification the code implements, not a reading off a
working credential. Treat it accordingly, and if you get one working, add a probe row.

## What Android needs on top, and why you cannot route around it

The Kampr **app** cannot use the browser's answer. Credential Manager will not run a ceremony for a
native app until it has confirmed that the app is allowed to hold credentials for that RP ID, and the
way it confirms that is Digital Asset Links:

> **Google's servers fetch `https://<rp-id>/.well-known/assetlinks.json` from the public internet**
> and look for the app's package name and signing certificate in it.

Read that twice, because every awkward consequence follows from it.

- **The phone never reads the file.** It asks Google. So "my phone can reach the node" is not the
  question being answered, and a node that serves the document perfectly to everything on the LAN is
  refused anyway.
- **A hostname that resolves publicly to a private address does not work.** This is the operator's
  own case, measured (#170): `kampr.oldug.com` serves the file with a 200, `application/json`, the
  right package and a fingerprint matching the release keystore exactly, and the node reports
  `tier 1, passkeys: true`. Google answers `ERROR_CODE_FETCH_ERROR`, because the public record points
  at `10.0.0.6`. Credential Manager then refuses with `RP ID cannot be validated`, which tells the
  owner nothing. **`security.passkeys` is necessary and not sufficient** — the party that actually
  decides is one the node never asks.
- **Tailscale, a VPN, split-horizon DNS and a LAN-only certificate all fail the same way**, for the
  same reason. They make the host reachable by you. The fetch is not by you.
- **Exposing it briefly is not a strategy.** Google caches a `statements:list` verdict for its own
  `maxAge` and does not track the `Cache-Control` you serve — measured at 600 s for a failure and
  between 600 s and 2974 s for successes on three real sites (#255). Open the hostname, let Google
  validate, close it again, and the verdict expires within the hour and the phone is refused.
- **A proxy that answers `/.well-known` itself will eat it** (#122). This is a separate failure with
  the same symptom, which is why `kampr doctor` fetches the file from this machine as well as asking
  Google: served here and unfetchable by Google is DNS, unserved in both places is the proxy.

### Can the RP ID point somewhere else?

Not anywhere. WebAuthn allows an RP ID that is **the origin's host or a registrable suffix of it**,
and nothing further. A node at `https://kampr.example.com` may use `kampr.example.com` or
`example.com`. It may not use `pk.example.com`, `example.net`, or any other host, and the browser is
the one that enforces this — `[auth] rp_id` in `config.toml` sets the value with no suffix check of
its own, so getting it wrong produces a refused ceremony rather than a configuration error.

That one allowance is the only real escape hatch, and it is worth understanding because it is
probably what you want:

> Set `[auth] rp_id = "example.com"` and it is **`example.com`'s** `/.well-known/assetlinks.json`
> that Google fetches and validates. The node itself stays wherever it is, private.

If you already have anything public on the apex — a site, a redirect, a landing page — serving one
extra static JSON file from it is the whole of the public exposure. `kampr doctor`'s check is aimed
at the RP ID rather than at the origin's host for exactly this reason, so it will interrogate the
right hostname when you do it.

**This has not been driven end to end.** The code path exists and is tested at the unit level, and
the design was written for this case, but no probe row records a passkey created against an apex RP
ID with the node on a private hostname. If you try it, that is a row worth adding.

Two things to know before you do. The credential is scoped to `example.com`, so it is offered on
every origin under that domain and shares the scope with any other WebAuthn relying party you ever
run there. And the file must stay served: Google re-validates when its cache expires, so this is a
permanent commitment to one public path, not a one-off.

### What the node serves

`GET /.well-known/assetlinks.json`, unauthenticated, built once when the router is built rather than
per request. It carries `delegate_permission/common.get_login_creds` and nothing else — Kampr claims
no URLs, so the app-link relation is deliberately absent. The package and fingerprints come from
`[android]` in `config.toml`, which defaults to `dev.kampr.app` and the release keystore's
certificate, so an operator running the published APK configures nothing. A source build signed with
your own keystore puts its own fingerprint there instead; blanking the section entirely makes the
node serve no document at all, which is a decision rather than a fault and `doctor` warns rather than
fails on it.

## What `kampr doctor` tells you

The `assetlinks` check does not read the local file and assume. It asks
`digitalassetlinks.googleapis.com` what it can see for the RP ID, and only if that goes badly does it
fetch the same URL from this machine as a second vantage point. What you get:

| You see | What it means |
|---|---|
| `ok … not in play: <origin> cannot do passkeys at all` | Below tier 1. Nothing reads the file, so there is nothing to fix, and the check says so once and stops |
| `ok … Google reads <file> and finds dev.kampr.app delegated to 1 certificate` | The party Credential Manager asks agrees with this node. This is the only green there is |
| `warn … could not ask Google` | No route to the validator from here. Unestablished, not broken — an unanswered question is never reported as a failure |
| `warn … answered <status> instead of a verdict` | Google's API, not your node. Re-run in a few minutes |
| `warn … [android] names no package and no usable certificate` | This node delegates to no app at all |
| `fail … Google cannot read <file> … refuses every passkey here with "RP ID cannot be validated"` | The private-host case. If this machine reads the file fine, the check says so and the fix it prints is DNS; if this machine cannot read it either, the fix it prints is the proxy |
| `fail … have drifted: Google reads X signed by Y, and this node names Z` | Somebody else's `assetlinks.json` is on that hostname. Google's copy is the one Android obeys |

**A green is up to about ten minutes stale, and a fix takes at least that long to show.** Every fix
this check prints says to wait rather than to re-run, because Google's cached verdict has to expire
first (#255).

The `tier` check answers the browser half separately, and on a Tier 0 node it names the specific
reason — an IP address is not a registrable domain, a hostname without a certificate is not a secure
context — rather than reporting a generic absence.

## What breaks if you change your mind

**A passkey does not survive a change of RP ID.** There is no migration path and no warning at
startup: the credential rows stay in the database and quietly authenticate nobody. Moving from
`kampr.example.com` to `example.com`, or to a different hostname entirely, means every enrolled
passkey is dead and every device re-enrols. This is why the threat model's advice is to set the
canonical origin *before* enrolling the first one, and it is close to the only irreversible decision
in a Kampr setup.

Everything else is reversible and cheap. Turning passkeys off is not a setting — it is what a node
already is until someone enrols one, and the pairing-code path never goes away or stops working
alongside them. Dropping the public `assetlinks.json` costs you Android app passkeys and nothing
else; the app falls back to the same pairing code the browser uses.

## If you are skipping this

That is a reasonable decision and the honest summary of it is: you keep bearer tokens with a 30-day
expiry, and you should get the middle rung of the ladder anyway. A hostname with a real certificate —
a DNS-01 wildcard on a name that never resolves outside your LAN is enough — ends the cleartext, and
that is the change that actually moves your exposure. Passkeys on top of it are a real improvement
against phishing and against a stolen token, and they are not the thing standing between the internet
and your shells. TLS is.
