#!/bin/sh
# CI only, in a fresh checkout whose dist/ holds just the placeholder. The release binary embeds
# the bundle, so Gradle-then-cargo is load-bearing; this proves the guard refuses an empty dist.
set -eu
cd "$(dirname "$0")/.."
if KAMPR_REQUIRE_BUNDLE=1 cargo build -p kampr-node > guard.log 2>&1; then
  echo "error: kampr-node built with an empty dist/ — the bundle guard is not working" >&2
  exit 1
fi
grep -q "no client bundle" guard.log
rm guard.log
