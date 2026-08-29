# Notifications

Phase 8. What a Kampr node sends, what a client may receive, and — where it cannot — what would
change that.

Every number here was measured on 2026-08-20 against **herdr 0.8.2, protocol 20**, in throwaway
named sessions torn down afterwards. Claims without a measurement behind them say so.

## The event this rests on

`pane.agent_status_changed` is the only signal that says an agent needs you. It was not
subscribed, and the reason was real: **herdr rejects a subscription to it without a `pane_id`, and
one invalid entry rejects the whole `events.subscribe` call** (probe #54). A node has many panes,
so one subscription for the session is not available; the list has to name every pane.

It now does. `kampr-core` subscribes to the session-wide topology kinds **plus one status entry
per agent pane**, and rebuilds the list whenever the agent-pane set moves. Three rules make that
safe:

- **Events poke the poll; they never replace it.** Every event ends in the same `session.snapshot`
  refresh the 3-second timer would have done, so a missed event costs one interval and never
  correctness. That is what makes a subscription safe to lose and safe to rebuild.
- **A resubscribe is debounced.** A workspace opening ten agent panes moves the set ten times.
  `resubscribe_min` (500 ms) collapses that into one socket and one subscribe.
- **A stale pane id is fatal, and expected.** A pane that closed between the snapshot and the
  subscribe answers `pane_not_found` and takes the whole call with it (probe #107) — the same
  all-or-nothing rule as a missing `pane_id`, at a different stage. It is a race, not a fault: the
  next pass re-derives the set from a fresh snapshot. An *existing* subscription survives its pane
  closing (probe #107), so only the initial call is exposed.

**Measured, against a real `claude` driven to a real permission prompt:** the event beat the
3-second poll by **1.38 s / 2.21 s / 2.58 s / 2.67 s / 2.84 s** over five runs — mean **2.33 s**.
That is the interval the whole triage story used to spend waiting.

## Server

| Piece | Where |
|---|---|
| VAPID keypair | `crates/kampr-push/src/vapid.rs`, generated at `kampr init`, `<state>/vapid.pem` at 0600 |
| Subscriptions and rules | `crates/kampr-auth/src/push.rs`, in the device database |
| Notification shape and batching | `crates/kampr-push/src/{note,batch}.rs` |
| Delivery | `crates/kampr-push/src/send.rs` — `web-push` builds and encrypts, `reqwest` posts |
| The blocked-pane watcher | `crates/kampr-node/src/push.rs` |

**The subscription store is the device store**, deliberately. A push subscription is a standing
invitation to wake a phone and is exactly as sensitive as the bearer token beside it, so revoking
a device has to end it. `Store::push_targets` is a join against live devices — revocation is a
`WHERE` clause, not a cleanup job somebody has to remember to run.

**Rotating the VAPID key invalidates every subscription already issued**, because a browser stores
the public half inside the subscription it hands back. So it is written once, at `kampr init`, and
loaded thereafter — including on a Tier 0 node, which cannot use it yet but will if it ever climbs
the ladder.

**The question is in the body.** The node already extracts it for the `pending` message (off the
screen, because Claude publishes nothing about a pending request until after it is answered —
probe #42), so a notification that only named the agent would be withholding what it has. Collie's
own architecture doc calls this its known gap. It earns its keep most on Android, where the OS may
hold the app long enough that a tap arrives before the tunnel is up: the body is all there is
until then.

**Simultaneous blocks are one notification.** A 900 ms collection window opens at the first change;
everything that lands inside it goes into one payload, split per subscription so a device that
muted one of three agents sees the other two rather than the whole batch or nothing. Three
notifications racing at a phone is how the feature gets turned off.

## The payload is the set, not the edge

One tag, one notification id: whatever arrives last is the only thing on the screen. So a payload
that names less than everything **silently unsays the rest** — and for as long as the node sent
only rising edges, it did, twice:

- A second agent blocking replaced the first one's notification and took it off the phone.
- A prompt answered anywhere else — at the desk, in the TUI, on another phone — sat there until
  somebody tapped it, because a falling edge was not an event anybody sent.

`kampr_push::Change` carries the whole outstanding set, plus `fresh` and `cleared` — which decide
only *whether it buzzes* and *who has to be told*. `watch_herd` emits on both edges and skips a
herd rebuild that did not move the set, which is also what keeps the per-pane `pending` read off
the three-second poll.

| The device's set | What it gets | Urgency |
|---|---|---|
| Gained a pane | The whole set, alerting | `high` |
| Shrank, still non-empty | The whole set, `alert: false` | `normal` |
| Emptied | `count: 0` — the payload that takes the prompt down | `normal` |
| Did not move | Nothing. A wake-up that repeats the screen buys nothing | — |

**A resync corrects a prompt; it never conjures one.** Both clients check whether one is showing
first: somebody who swiped the notification away has already dealt with it, and posting a quieter
copy of what they dismissed is the app arguing with them.

**The clear degrades rather than breaking.** Payload `v` is 2, and it is additive: a client that
predates it reads `title` and `body` and shows the clear as an ordinary notification, which under
the same tag *replaces* the stale prompt instead of leaving it. The degradation is a notification
to dismiss, never a prompt that lies. Old Android APKs are the reason this matters — they are on
real phones and they only ever read three fields.

**The service worker shows the clear before closing it.** A browser is entitled to post its own
"this site has been updated in the background" when a push displays nothing, and that notice is
worse than the one it replaces because nobody here wrote it. Showing under the same tag replaces
whatever is there, so the close that follows removes one entry rather than two. *Which* browsers
do this, and after how many silent pushes, is **not measured on this rig** — the mitigation is
unconditional precisely because the answer is unknown.

**And the client takes its own prompt down.** A notification is a summary of the moment it was
sent; a running client sees the herd first-hand and is fresher than any push. `AppState` reconciles
on every herd update, guarded on `known` — an unloaded herd has no blocked panes either, and
reconciling against it would take down the very notification whose tap opened the app. It only ever
*removes*: rewriting a shrunken summary would mean reproducing the node's title and body shaping in
every client, and the resync push already does that from the one place that holds the questions.

## Tiers — what is possible where

Web Push needs a **secure context**, and plain HTTP on a LAN IP is not one (findings §3.7). This
is not a Kampr limitation and no amount of client code routes around it.

| Tier | Origin | `security.push` | `caps.push` | What the client shows |
|---|---|---|---|---|
| 0 | `http://192.168.1.24:8790` | `false` | `false` | Nothing. `unlocks` says a hostname and a certificate would add it |
| 0 (loopback) | `http://127.0.0.1:8790` | `true` | `true` | Everything — a loopback IP **is** a secure context, which is why this is testable with no domain |
| 1–3 | `https://kampr.home.example.com` | `true` | `true` | Everything |

`security.push` says whether the *origin* allows it. `caps.push` says whether this *node* can
actually do it — secure context **and** a VAPID key **and** `push.enabled`. A client hides the
control on either.

## Client

- **Service worker** at `/sw.js`, scope `/`. Four jobs and no others: show the push, open the
  right pane on a tap, warm a small cache on the way to both, and serve those two warm URLs back
  to the page. It deliberately does **not** proxy the app's own fetches — the wasm bundle is
  served immutable and the browser already caches it.
- **The token reaches the worker over `postMessage` and is kept in IndexedDB.** A service worker
  cannot read `localStorage` and outlives every page. A cookie would have taken fewer lines and
  added a CSRF surface to a node that has none.
- **Warm resume**: on push the worker fetches `/api/node` and `/api/warm?pane=…` — the herd plus
  that pane's outstanding question, a few kilobytes — so the tap opens onto data. It is not the
  grid: reproducing the wire's per-connection style interning outside a live socket would be a
  second encoder, and the socket delivers the real one within a second of the tap.
- **Deep link**: one blocked pane opens that pane in its conversation view, which is the view an
  answer can be given from without leasing a terminal. A batch opens the triage list, because
  picking one of three for the user is picking wrong two times in three.
- **Triage list**: `KamprStore.triage()` — blocked panes, newest first, above the herd on every
  breakpoint. `KamprStore.blocked()` had no callers until now.
- **iOS**: Web Push works **only** for a Home Screen web app. A Safari tab can neither subscribe
  nor be told why by the push API, so `PushCapability.NeedsHomeScreen` is detected from
  `navigator.standalone` and the screen prompts for Add to Home Screen rather than offering a
  button that fails inside the permission call. `index.html` carries the `apple-mobile-web-app-*`
  meta tags, without which Add to Home Screen produces a bookmark that can never be notified.

`sw.js`, `index.html` and `manifest.webmanifest` are served `no-store`. Everything else in the
bundle is content-hashed and immutable — but an immutable service worker is one that never
updates, and the browser would keep running it against a new node forever.

## Android, natively — UnifiedPush

**Decision: UnifiedPush, not FCM.** One sentence: a Kampr node is already a server the user runs,
and UnifiedPush 3.0 carries the same RFC 8291 encryption and VAPID that the browser does — so the
node's sender is *unchanged*, there is no Google project, no `google-services.json`, and no
per-app secret in the node's config, which is the whole point for the people who self-host a
terminal bridge.

**The server half is done and needs nothing further.** A distributor's endpoint is a Web Push
endpoint: `POST /api/push/subscribe` with `kind: "unifiedpush"` stores it, and `kampr-push` sends
to it byte for byte identically. `kind` is a label for the device list, never a branch.

**The client half needs infrastructure the user installs**, which is why it is documented rather
than assumed:

1. Install a UnifiedPush distributor on the phone — [ntfy](https://ntfy.sh) is the usual choice
   and can point at a self-hosted ntfy server, or Sunup for a pure-FOSS one.
2. In the Kampr Android app, register with the distributor (`org.unifiedpush.android:connector`,
   `UnifiedPush.register(context, instance)`).
3. On `onNewEndpoint`, POST the endpoint and the connector's own P-256/auth keys to
   `/api/push/subscribe` with `kind: "unifiedpush"` — the same body the browser sends.
4. On `onMessage`, the payload is the same JSON `kampr-push` produces, already decrypted by the
   connector. Render it, and open `pane` on a tap — or, on `count: 0`, cancel the notification
   instead of rendering anything.

Until a distributor is installed there is nothing to register with, and the app says so rather
than failing quietly. `createPushPlatform()` on Android returns `NoPush` today.

`PushCapability.Unsupported` is also what the desktop JVM build reports, on the grounds that a
desktop Kampr is already on the screen the herd is running on.

## `notification.show` — the reverse direction

Probe #50: the node can raise a toast on the **desktop**. One place it is not noise:

- **A pairing confirmation.** When a device redeems a code, the operator watching the console sees
  it happen on the same screen the code was printed on. A pairing nobody expected is exactly the
  one worth noticing, and this is the only channel that reaches a person who is not holding the
  phone.

There was a second: a **"Tell the desk"** button on a pane header, which announced that a remote
device was about to type into a terminal somebody might be sitting at. It is gone. On a node run
as a service — a headless herdr, which is what the plugin and the systemd unit both produce — it
could only ever answer `no_foreground_client`, so the button's every outcome was "No desk". And
the job it was invented for is now done passively and better by **watcher presence**, which shows
that another client has the pane open without anyone pressing anything. The `notify` client
message and its `notified` reply went with it.

Three rules, in `crates/kampr-node/src/toast.rs`:

- **Always attributed.** The node prefixes what raised it. An unattributed toast on an operator's
  desktop is a phishing surface.
- **Rate limited** (5 s). Anything that can put arbitrary text on someone's screen as fast as it
  likes is a denial of service against the person.
- **Control characters are stripped, not escaped.** The text is rendered by a TUI.

**It is answered honestly.** `notification.show` returns `{shown, reason}`, and on a herdr with no
attached client it returns `shown: false, reason: "no_foreground_client"` (probe #77).

## What is not proved

- **A real phone.** Everything here was measured on a desktop browser against a loopback origin.
  The Android emulator has no distributor, and no physical device was available.
- **iOS.** The Home Screen detection and the A2HS prompt are written against the documented
  behaviour and have not been run on an iPhone. Nothing else in this repo can be, either.
- **A public push endpoint under load.** One subscription, one node, one LAN.
- **What a browser does with a push that displays nothing.** The `quench` idiom in `sw.js` is
  written so the answer does not matter; the answer itself is unmeasured.
- **The Android resync and clear on a real device.** `BlockedNotificationTest` asserts all three
  paths against a real `NotificationManager`, but instrumented tests need a device and none was
  available — the same gap as everything else in this section.
