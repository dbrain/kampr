# Android release

Signed release APK for `dev.kampr.app`, distributed through [kobup](../../tinyfiddler/kob/kobup).
Everything below runs from the repository root.

```bash
make android-release        # signed, minified APK + signature and font proof
make android-publish        # bump version, rebuild, upload to kobup, push to devices
```

## Prerequisites

| | |
|---|---|
| JDK | 21+ (the build pins `jvmToolchain(21)`; the host JDK may be newer) |
| Android SDK | `~/Android/sdk`, path in `client/local.properties` (`sdk.dir=`) — never committed |
| SDK packages | `platforms;android-36`, `build-tools;36.0.0`, `platform-tools` |
| Signing | a release keystore, see below |
| Publishing | `KOBUP_TOKEN`, see below |

**`GRADLE_HOME` on this machine points at a broken Gradle 8 install (probe #67).** Every Gradle
invocation must be `env -u GRADLE_HOME ./gradlew …`. The Makefile already does this; if you type
`./gradlew` by hand, unset it yourself.

The Gradle build root is `client/`, not the repository root. `.kobup.json`, `version.properties`,
`.env` and `gradlew` all live there, because that is what `${rootDir}` resolves to inside the kobup
helper.

## What "release" means here

`:androidApp` release differs from debug in four ways that all have teeth:

- **R8 minification and resource shrinking** are on. `androidApp/proguard-rules.pro` carries the
  keep rules kotlinx.serialization (synthetic `$serializer` classes and companion accessors that
  nothing references by name), Ktor (engine container picked out of a `ServiceLoader`) and Compose
  Multiplatform resources need. Without them the app compiles, installs, and dies the first time it
  decodes a wire frame.
- **Assets are staged by hand.** CMP 1.11.1 emits no Android assets for a
  `com.android.kotlin.multiplatform.library` target, so `stageComposeResourceAssets` copies
  `shared/src/commonMain/composeResources` into the APK itself (probe #64). This failed silently
  once already: Android fell back to system sans and a benchmark measured the wrong font.
- **`verifyReleaseApkResources` reads the packaged APK back** and fails the build if any staged
  resource is missing from it. It is wired as `finalizedBy` on `assembleRelease`, so a regression
  cannot reach kobup — the helper shells out to `assembleRelease` and checks its exit code.
  `verifyReleaseBundleResources` does the same for the AAB.
- **Signing is mandatory.** `checkReleaseSigning` runs before `packageRelease` and fails loudly
  rather than letting AGP emit `androidApp-release-unsigned.apk`.

`minSdk` is **26**. Oreo is the floor that gives the whole `java.time`/NIO surface without
desugaring and adaptive icons, and it covers every Android TV box worth sideloading onto. The
client is native Compose — no WebView, no wasm, no Web Push — so nothing here is arguing for a
lower floor. It costs about 2% of live devices.

## Keystore

```bash
make android-keystore
```

Generates `~/.android-keystores/kampr-release.jks` (PKCS12, RSA 4096, 30 years, alias `kampr`) with
a random password, and appends the four properties to `~/.gradle/gradle.properties`:

```properties
kamprReleaseStoreFile=/home/you/.android-keystores/kampr-release.jks
kamprReleaseStorePassword=…
kamprReleaseKeyAlias=kampr
kamprReleaseKeyPassword=…
```

Each has an environment-variable fallback for CI: `KAMPR_RELEASE_STORE_FILE`,
`KAMPR_RELEASE_STORE_PASSWORD`, `KAMPR_RELEASE_KEY_ALIAS`, `KAMPR_RELEASE_KEY_PASSWORD`. The Gradle
property wins when both are set. The target refuses to overwrite an existing keystore.

### Back up

Two things, off this machine, in whatever you use for secrets:

1. `~/.android-keystores/kampr-release.jks`
2. the store/key password

**Losing either is terminal.** Android identifies an app by `(packageName, signing certificate)`.
A Kampr APK signed with a different key is a different app to every device that already has one:
kobup will offer the update, the install will fail with `INSTALL_FAILED_UPDATE_INCOMPATIBLE`, and
the only recovery is to walk to each phone, tablet and TV box, uninstall (losing its local state)
and reinstall. There is no Play Store key rotation to fall back on for sideloaded apps. The APK is
signed with v2 and v3 schemes; v3 at least makes a *proactive* rotation possible in future, but only
if you still hold the current key.

The keystore is never in the repository. `.gitignore` covers `*.jks`, `*.keystore`, `.env` and
`local.properties`.

### The signing certificate is also the app's identity to a passkey

Android's Credential Manager runs no WebAuthn ceremony for a native app unless the relying party —
your own node, at your own domain — publishes a Digital Asset Links file naming the app. So the node
serves one at **`/.well-known/assetlinks.json`**, unauthenticated (it has to be readable before any
credential exists) and built once at startup, delegating exactly one relation:
`delegate_permission/common.get_login_creds`. It is deliberately *not* an app-links file: an app link
declares its hosts in the manifest at build time and every operator's node is at a different one, so
Kampr claims no URL and enrolls through the in-app scanner instead.

What it names comes from `[android]` in `config.toml`:

```toml
[android]
package = "dev.kampr.app"
fingerprints = ["A0:8A:21:84:…"]   # SHA-256 of the signing certificate
```

The default is the release keystore's own certificate — the one every APK kobup ships is signed
with — so an operator who installed Kampr configures nothing. It is a *default* rather than a
constant because two other builds exist and neither is signed with it: a **debug** build carries the
machine's own `~/.android/debug.keystore`, and a build from source carries whatever keystore made it.
Both are refused by a stock node, and the refusal is otherwise a shrug, so the app reads its own
certificate off `PackageManager` and prints the line to paste:

> This node names dev.kampr.app but not the certificate this build is signed with. Add it to
> `[android] fingerprints` in its config.toml: "18:77:9D:…"

Two more things follow from the same certificate. Credential Manager does not sign an `https://`
origin into its client data — it signs `android:apk-key-hash:<base64url of that SHA-256>` — so the
node adds that origin to the WebAuthn engine, derived from the same configured fingerprints
(`kampr-auth/src/android.rs`). And `webauthn-rs`'s general passkey options are a ceremony Android
cannot perform, so `/auth/webauthn/register/start` takes `{"platform":"android"}` and answers with a
discoverable platform credential and no `credProtect`, which is what the crate's
`workaround-google-passkey-specific-issues` feature exists for. A browser's options are unchanged.

`cargo test -p kampr-node --test android_passkeys` reads the fingerprint back out of the built APK
with `apksigner` (or out of the keystore with `keytool`) and fails if the default no longer matches;
absent both it skips loudly, and `KAMPR_ANDROID_CERT=1` turns that skip into a failure.

## Building

```bash
make android-release     # APK  → client/androidApp/build/outputs/apk/release/androidApp-release.apk
make android-bundle      # AAB  → client/androidApp/build/outputs/bundle/release/androidApp-release.aab
make android-install     # build, then adb install -r onto the attached device
make android-test        # instrumented tests on the attached device
```

`android-release` prints the `apksigner verify --verbose` result and the packaged font count after
the build, so a broken signature or a lost asset is visible without a second command.

The AAB comes free from AGP and is verified the same way, but kobup distributes APKs — the AAB is
only useful if Kampr ever goes to a store.

### Tests

`client/androidApp/src/androidTest` holds the device-side tests. One opens every `composeResources`
font out of the installed APK's `AssetManager` and checks the sfnt magic — the runtime half of probe
#64, where `verifyReleaseApkResources` is the artefact half. The others cover what only a device can
answer: that Credential Manager is reachable once an Activity is attached, what certificate this
build is signed with, that `CAMERA` is declared, and that the QR the desktop draws decodes on the
device. With `KAMPR_NODE=https://…` the asset-links check runs against that node as well.

Instrumented tests run against the **debug** variant. `testBuildType = "release"` was tried and
abandoned: the instrumentation APK deliberately does not duplicate classes the app under test
already ships, so running it against a minified app needs a growing list of keep rules for the
runner's own dependencies (`androidx.tracing.Trace`, then `kotlin.LazyKt`, …) that exist only to
serve the tests. The release APK is covered instead by the packaging assertion plus an actual
install-and-launch.

## Publishing to kobup

Kobup is the internal distribution system: it hosts the APK, tracks versioned releases per channel,
and pushes an FCM refresh so sideloaded devices pick the update up.

```bash
make android-publish     # = cd client && env -u GRADLE_HOME ./gradlew :androidApp:publishToKobup
```

Kampr's Android module is `:androidApp`, so the invocation is `:androidApp:publishToKobup`, not the
`:app:publishToKobup` in the helper's own header.

### Configuration

`client/.kobup.json` — committed, no secrets:

```json
{
  "projectSlug": "kampr",
  "projectName": "Kampr",
  "packageName": "dev.kampr.app",
  "channel": "stable",
  "server": "https://kobup.oldug.com"
}
```

`client/version.properties` — committed; the helper owns it:

```properties
versionCode=1
versionName=0.1.0
```

### The token

`KOBUP_TOKEN` is the CI token from `kobup login`; the CLI leaves it in `~/.config/kobup/config.json`.
It is never in the repository and never in a Gradle file. Two ways to supply it:

- **`.env`** in `client/`, read by the `co.uzzu.dotenv.gradle` plugin applied in
  `client/build.gradle.kts`. Copy `client/.env.template` to `client/.env` and fill it in. `.env` is
  gitignored. This is the convenient local option, and the one the helper reads first.
- **Environment variable** — `KOBUP_TOKEN=… make android-publish`. This is the CI option, and the
  fallback when no `.env` exists.

### What the task does

In order, in one Gradle task:

1. Refuses to run if `git status --porcelain -- '*.kt'` is non-empty. Commit or stash first — the
   version name it is about to stamp is a git hash, so a dirty tree would produce an APK that
   claims to be a commit it is not. Untracked `.kt` files count as dirty.
2. Requires `KOBUP_TOKEN`.
3. Bumps `version.properties`: `versionCode` +1, `versionName` patch +1, then suffixes
   `+<short git hash>` — `0.1.0 (1)` becomes `0.1.1+bd7e8ba (2)`. Both land in the APK manifest.
4. Runs `client/gradlew :androidApp:assembleRelease` as a **nested** Gradle build, and restores
   `version.properties` if it fails.
5. `POST {server}/api/v1/admin/projects` to create the project if it does not exist (idempotent).
6. `POST {server}/api/v1/ci/projects/kampr/release` — multipart `apk`, `channel`, `publish=true`.
7. `POST {server}/api/v1/ci/projects/kampr/push-refresh` to wake the devices.

So the normal sequence is: commit, then publish.

```bash
git commit -am "…"
make android-publish
git commit -am "release 0.1.1"   # version.properties changed under you
```

## Rolling back a bad release

The helper only goes forward. To pull a release back:

1. **Unpublish in the kobup web UI** (`https://kobup.oldug.com`) — open the project and unpublish
   the bad release. Devices stop being offered it. Kobup keeps the last
   `MAX_RELEASES_PER_CHANNEL` (default 10) per channel, so the previous APK is still on the server
   and becomes latest again.
2. **Or roll forward**, which is what a device that already installed the bad build needs anyway.
   Android will not downgrade an installed app to a lower `versionCode`, so a "rollback" that
   reaches devices has to be a *higher* `versionCode` carrying the older code:

   ```bash
   git revert <bad commit>
   git commit
   make android-publish            # new, higher versionCode; same code as before the bad commit
   ```

3. If a bad build is only on your desk, `adb uninstall dev.kampr.app` and reinstall the good APK.

Test a risky build on the `beta` channel first — set `"channel": "beta"` in `.kobup.json`, publish,
and only devices subscribed to beta see it.

## Failure modes

| Symptom | Cause |
|---|---|
| `Cannot find module 'gradle-public-api-legacy'` | `GRADLE_HOME` (probe #67). Use `env -u GRADLE_HOME ./gradlew`. |
| `Release signing is not configured` | No keystore properties. `make android-keystore`. |
| `Release keystore … does not exist` | Properties point at a keystore that is gone. **Restore from backup — do not generate a new one.** |
| `… is missing N of 21 compose resources — probe #64 has regressed` | `stageComposeResourceAssets` stopped feeding the variant. Check `variant.sources.assets` wiring in `androidApp/build.gradle.kts`. |
| `Uncommitted .kt changes` | The kobup guard. Commit or stash. |
| `KOBUP_TOKEN not set` | No `client/.env` and no environment variable. |
| `INSTALL_FAILED_UPDATE_INCOMPATIBLE` on a device | That device holds an APK signed with a different key — a debug build, or a pre-keystore build. Uninstall it there. |
| App starts, shows a blank dark screen forever | `KamprTheme` gates first paint on font resolution (probe #65). A blank screen means the fonts did not load — check the APK's `assets/composeResources/`. |
