#!/usr/bin/env bash
# A test task's cache key is its classpath, not the machine that ran it, so a result stored by one
# runner is restored onto another with different fonts, a different locale and no display and
# reported as a pass with nothing executed. Four `:terminal:jvmTest` tests failed on every runner
# for weeks behind `FROM-CACHE`. The init script makes test tasks run; the greps are here because
# an init script that silently stopped applying would put the hole straight back.
#
# Not `build`: that packages the *release* variant, and `checkReleaseSigning` correctly refuses to
# produce an unsigned APK without the keystore, which stays on the operator's machine.
set -euo pipefail
cd "$(dirname "$0")/../client"
# The QR is decoded by an independent decoder rather than asserted against the bitmap that
# produced it. Without zbar the test skips, and this makes that skip a failure.
export KAMPR_QR_DECODE=1
# A workstation fixes what ktlint can fix before checking; CI only checks, so a push that skipped
# this still fails rather than being formatted on a runner nobody reads.
if [ -z "${CI:-}" ]; then
  env -u GRADLE_HOME ./gradlew ktlintFormat --continue -q
fi
log=$(mktemp)
trap 'rm -f "$log"' EXIT
env -u GRADLE_HOME ./gradlew -I gradle/tests-must-run.init.gradle.kts \
  check :webApp:stageNodeBundle :androidApp:assembleDebug 2>&1 | tee "$log"
# A closed set of test-*execution* task names, not a `Test` suffix: `compileAndroidHostTest`,
# `javaPreCompileDebugUnitTest` and `lintAnalyzeDebugAndroidTest` all end in `Test` and are
# honestly cacheable.
if grep -E '^> Task :[^ ]*:(jvmTest|wasmJsTest|wasmJsBrowserTest|testAndroidHostTest|test) FROM-CACHE$' "$log"; then
  echo "error: a test task was served from the build cache — this run proved nothing" >&2
  exit 1
fi
for task in :shared:jvmTest :terminal:jvmTest :shared:testAndroidHostTest; do
  grep -qx "> Task $task" "$log" || { echo "error: $task did not run, so nothing it guards was checked" >&2; exit 1; }
done
test -s ../crates/kampr-node/dist/index.html || { echo "error: no staged index.html" >&2; exit 1; }
ls ../crates/kampr-node/dist/*.wasm >/dev/null
