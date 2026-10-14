#!/usr/bin/env bash
# CI only: starts `herdr server` in the default session and a node on a fixed port, which on a
# workstation would be the operator's own herd. LiveNodeTest is the only real-client-against-
# real-node test, and KAMPR_LIVE makes its skip a failure.
set -euo pipefail
: "${KAMPR_CONFIG_DIR:?}" "${KAMPR_STATE_DIR:?}"
cd "$(dirname "$0")/.."
herdr server &
for _ in $(seq 1 60); do [ -S "$HOME/.config/herdr/herdr.sock" ] && break; sleep 1; done
./target/debug/kampr init --bind 127.0.0.1:8794 --name ci
./target/debug/kampr serve &
for _ in $(seq 1 60); do curl -fsS http://127.0.0.1:8794/api/node >/dev/null && break; sleep 1; done
code=$(./target/debug/kampr pair | tr -d '[:space:]')
token=$(curl -fsS -X POST http://127.0.0.1:8794/auth/pair \
  -H 'content-type: application/json' \
  -d "{\"code\":\"$code\",\"device_name\":\"ci\"}" | jq -er .token)
KAMPR_LIVE=1 KAMPR_URL=http://127.0.0.1:8794 KAMPR_TOKEN="$token" \
  env -u GRADLE_HOME ./client/gradlew -p client \
    -I "$PWD/client/gradle/tests-must-run.init.gradle.kts" :shared:jvmTest \
    --tests 'dev.kampr.shared.LiveNodeTest.theClientReadsTheAuthSurfaceOfARealNode' \
    --tests 'dev.kampr.shared.LiveNodeTest.preferencesWrittenOnOneConnectionComeBackOnTheNext'
