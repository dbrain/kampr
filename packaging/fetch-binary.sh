#!/bin/sh
# Runs at `herdr plugin install` time, with no plugin runtime env injected. All it does is put the
# binary in the plugin root — install.sh owns naming, downloading and verification for both paths.
set -eu

ROOT="$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)"
exec env KAMPR_PREFIX="$ROOT/bin" KAMPR_MODE=plugin sh "$ROOT/packaging/install.sh"
