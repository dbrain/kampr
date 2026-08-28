# The toolchain this builds with

Every version below was used to build and test the tree as it stands. Where a version is a floor,
it says so; where it is exact, something breaks below it and the reason is given.

Until this file existed, "whatever was installed last time" was doing this job, which is fine on one
machine and useless on a fresh one. A dependency pass stalled for an afternoon on exactly that.

## Building the node

| | | |
|---|---|---|
| Rust | **1.90+** | `rust-version` in the workspace manifest |
| Herdr | **0.8.2+** (protocol 20) | the only version anything here is verified against |

## Building the client

| | | |
|---|---|---|
| JDK | **21, exactly** | `jvmToolchain(21)` is pinned in every module. Gradle can provision it — see the note on `foojay` below — but the version is not a floor: 26 will not do |
| Gradle | **9.7.1**, via the wrapper | Never invoke a system `gradle`. A `GRADLE_HOME` on your `PATH` **silently overrides the wrapper**, so every command here goes through `env -u GRADLE_HOME ./gradlew` — which is what the `Makefile` does and why |
| Kotlin | 2.4.10 | |
| Compose Multiplatform | 1.11.1 | |
| AGP | 9.3.1 | |

Gradle 9.7.1 turns toolchain auto-provisioning without a declared repository into a **Gradle 10
error**, so `settings.gradle.kts` carries `org.gradle.toolchains.foojay-resolver-convention`.

### `.env`, and the plugin that used to read it

`co.uzzu.dotenv` 4.0.0 calls `Project.getProperties`, which errors under Gradle 10, and 4.0.0 is the
last release — so there was nothing to bump to. **It is gone.** Nothing in this tree ever used it:
release signing goes through `configValue` in `androidApp/build.gradle.kts`, which is
`providers.gradleProperty(…).orElse(providers.environmentVariable(…))`, and `env.` appears in no
`.kts` file here.

The one real consumer is the **kobup publish helper**, which lives outside this repository
(`…/kob/kobup/gradle/publish-to-kobup.gradle`) and is pulled in by `apply(from = …)` from
`androidApp/build.gradle.kts`. It captures the token at configuration time:

```groovy
def capturedToken = null
try { capturedToken = env.fetchOrNull("KOBUP_TOKEN") } catch (ignored) {}
…
def token = capturedToken ?: System.getenv("KOBUP_TOKEN")
if (!token) throw new GradleException("KOBUP_TOKEN not set. Add it to .env or export it.")
```

Deleting the plugin alone would have degraded *quietly*: the `try` swallows the missing `env` and CI
still works through `System.getenv`, but `client/.env` — the way this machine actually publishes —
would have stopped being read, with no message anywhere.

So the extension is now ours. `client/gradle/dotenv` is a small included build contributing the
settings plugin `dev.kampr.dotenv`, applied from `client/settings.gradle.kts`. It registers an `env`
extension on **every** project through `gradle.lifecycle.beforeProject` — the Isolated-Projects-safe
hook, not `allprojects` — exposing exactly the one method the helper calls:

| | |
|---|---|
| `env.fetchOrNull(String)` | `client/.env` first, then the environment variable of the same name, then `null` |

Both reads go through `providers.fileContents(…)` and `providers.environmentVariable(…)`, so the
configuration cache tracks them and nothing touches `Project.getProperties`. `.env` is absent on a
fresh checkout and that is not an error: `fetchOrNull` returns `null`, and the helper's own
`KOBUP_TOKEN not set. Add it to .env or export it.` is the message the operator sees. `.env.template`
is the documented example, and `.env` is git-ignored (`.gitignore:20`) — a token has never been in
this repository and must not be.

`client/gradle/dotenv/src/test/kotlin/.../DotEnvPluginTest.kt` is five TestKit builds that reproduce
the helper's call shape verbatim — a Groovy script `apply(from …)`'d into a subproject, calling
`env.fetchOrNull("KOBUP_TOKEN")` inside the same `try`/`catch` — because the shape is the contract
and the only thing that reads it is not in this tree. `client/build.gradle.kts` hangs the root
`check` off that build, so `./gradlew build` runs them.

**Gradle 10 status: no known blocker left.** `./gradlew help --warning-mode all` from `client/` and
from `client/gradle/dotenv` both report nothing. What has *not* been exercised is a real
`publishToKobup` against a real kobup server — see `docs/07-android-release.md`.

## Android

| | | |
|---|---|---|
| `cmdline-tools` | **23+** | |
| SDK platform | **`android-37.0`** | note the name — API 37 platforms are `android-37.0`/`37.1`, **not** plain `android-37` |
| Build tools | **37.0.0** | |
| Platform tools | 37.0.1 | |
| Emulator | 37.1+ | |
| System image | `google_apis_playstore;x86_64` API 37 | needed to test the `targetSdk` 37 behaviour below |
| `compileSdk` / `targetSdk` / `minSdk` | 37 / 37 / 26 | |

`minSdk 26` is deliberate: it buys the whole `java.time`/NIO surface with no desugaring, and adaptive
icons.

Two things about cmdline-tools 23 that cost time if you meet them cold. It **retires `sdkmanager`**
in favour of a new `android` CLI — the old name still works, prints a deprecation and delegates. And
`android sdk install` **fails with a `Storage.saveArchive` stack trace when given several packages at
once**; install them one per invocation.

### `targetSdk` 37 needs `ACCESS_LOCAL_NETWORK`, and getting this wrong is invisible

From `targetSdk` 37, Android enforces a local-network permission, and Kampr is precisely the app it
applies to: a self-hosted node is reached over plain `http` at a private address. Without the
permission the connection **times out after ten seconds with no permission error** — indistinguishable
in logs from a node that is simply down.

The restriction keys on whether an address is **on-link**, not on whether it is RFC1918. From an
emulator the host's LAN address is routed via the gateway and is *not* classified local, so it
answers fine without the permission — while `10.0.2.2` is on-link and does not. **Testing only the
LAN address produces a false pass**, and on a real phone sharing Wi-Fi with the node that same
address *is* on-link. Test an on-link destination or you have proved nothing (probes #100, #101).

Instrumentation cannot enable the compat change on itself — `am compat enable` kills the process it
names — so the test asserts the manifest declaration and the real refusal is forced from a shell
outside the app (probe #102).

## Optional, per task

| | |
|---|---|
| `zbar-tools` | decodes the pairing QR in `QrDecodeTest`. Absent, the test skips loudly; `KAMPR_QR_DECODE=1` turns that skip into a failure, which is what CI sets |
| `cross` | aarch64 release builds |
| `cosign` | verifies a release signature at install time. **Required to install from a published release** — the installer refuses rather than falling back to the checksums, which came from the same server as the tarball |
| `adb` | `make android-install`, `make android-test` |
| `apksigner` / `keytool` | the asset-links test reads the **release certificate's** SHA-256 off the built APK (falling back to the keystore) and asserts the node would name it. Absent, the test skips loudly; `KAMPR_ANDROID_CERT=1` turns that skip into a failure |

`make android-test` takes an optional `KAMPR_NODE=http://…` — with it, the suite additionally proves
the app reaches a plain-http node on a private address, which is the `ACCESS_LOCAL_NETWORK` path
above. Without it that test skips rather than failing on a machine with no node.

## Android passkeys need the node to name this build

Credential Manager will not create a passkey unless the relying party — the node, at the operator's
own domain — serves a Digital Asset Links file naming the app's package and signing certificate. The
node does that at `/.well-known/assetlinks.json`, defaulting to the release certificate, so an
operator installing the kobup APK configures nothing.

**A debug build or a build-from-source is signed with a different key and will be refused.** The app
reads its own certificate off `PackageManager` and, when a ceremony fails, fetches the node's
asset-links file; if this build is not named there, it replaces the error with the `[android]`
config lines to paste. So the failure is self-describing rather than mysterious — but it is a
failure, and it is expected on any build the operator did not install from a release.

Two things the file alone does not buy, both recorded as probes: Credential Manager signs an
`android:apk-key-hash:` origin rather than an `https://` one (#113), and `webauthn-rs`' generic
passkey options describe a ceremony Android cannot perform (#114). Both are handled node-side.

`kampr doctor` reports whether the origin actually serves the file, because the node always builds
it correctly and the thing that breaks is the path to it — a proxy with its own `/.well-known`
location block reads as a perfectly healthy node right up to the moment a ceremony is refused
(#122). Below Tier 1 the check stays quiet: there is no ceremony to fail.

Note that **passkey creation cannot be verified on a stock emulator** — an AVD with no Google
account has no credential provider at all (#116). It has therefore never been done anywhere; run it
once against a real phone.

## The release keystore

`../kampr-android-keys/kampr-release.jks` — beside the repo rather than hidden in `$HOME`, so it
is visible to whatever backs this machine up — with its passwords in `~/.gradle/gradle.properties`.
`make android-keystore` creates it and **refuses to overwrite an existing one**, because replacing it
orphans every device that has ever installed Kampr — no update path, by kobup or by hand, short of
uninstalling everywhere. Back up both the keystore and its password, off the machine.

## The version number is a release artefact

`version` in `[workspace.package]` is what `kampr --version` prints, what every node puts on the
wire as `build`, and what `kampr update` compares against the release tag. **Bump it to the tag
before tagging.**

It was not, for `v0.1.1`: the published `kampr-x86_64-unknown-linux-musl.tar.gz` for that tag
prints `kampr 0.1.0`, because the crates were never bumped and nothing sets `KAMPR_BUILD`. That
was invisible while nothing compared the two numbers. It is not invisible now — a release whose
binary reports the previous version tells every node in the herd, permanently, that it is one
release behind, and taking the update does not clear it. The workspace version is set to `0.1.1`
here so `build` and the published tag agree again.

The durable fix is either this line, bumped with the tag, or `KAMPR_BUILD` exported from the
release workflow — `crates/kampr-node/src/state.rs` already prefers `KAMPR_BUILD` over
`CARGO_PKG_VERSION`, so setting it in `release.yml` would make the tag the single source.

## Updating an installed node

`kampr update` embeds `packaging/install.sh` with `include_str!` and runs it with
`KAMPR_MODE=update` and `KAMPR_PREFIX` set to the directory the running binary is in. Two
consequences worth knowing:

- **The verifier is never downloaded.** Fetching `install.sh` from the release would mean whoever
  can serve a tampered binary can serve a verifier that accepts it. The embedded copy came out of
  the release the operator already verified.
- **The installer in the tree is the one in the binary**, so a change to `packaging/install.sh` is
  a change to what `kampr update` does, and `crates/kampr-cli/tests/update_cli.rs` exercises it
  against a release built on disk, served by a `curl` and a `cosign` on `PATH` that answer only
  for the canonical release base — so the URL the command asks for is asserted, not assumed.

`cosign` is required here for the same reason it is required in `install.sh`: absent, there is
nothing left that says who built the tarball, and a checksum served beside it does not.
`KAMPR_ALLOW_UNVERIFIED` and `KAMPR_BASE_URL` are deliberately *not* inherited — not by the
installer `kampr update` runs, and not by `packaging/fetch-binary.sh` on the `herdr plugin install`
path.
