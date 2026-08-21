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
error**, so `settings.gradle.kts` carries `org.gradle.toolchains.foojay-resolver-convention`. The
remaining blocker for Gradle 10 is `co.uzzu.dotenv` 4.0.0 calling `Project.getProperties`; 4.0.0 is
the latest release, so there is nothing to bump to yet.

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
| `cosign` | verifies a release signature at install time; the installer says so rather than pretending when it is missing |
| `adb` | `make android-install`, `make android-test` |

`make android-test` takes an optional `KAMPR_NODE=http://…` — with it, the suite additionally proves
the app reaches a plain-http node on a private address, which is the `ACCESS_LOCAL_NETWORK` path
above. Without it that test skips rather than failing on a machine with no node.

## The release keystore

`~/.android-keystores/kampr-release.jks`, with its passwords in `~/.gradle/gradle.properties`.
`make android-keystore` creates it and **refuses to overwrite an existing one**, because replacing it
orphans every device that has ever installed Kampr — no update path, by kobup or by hand, short of
uninstalling everywhere. Back up both the keystore and its password, off the machine.
