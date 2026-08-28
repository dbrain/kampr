#!/bin/sh
# Runs at `herdr plugin install` time, with no plugin runtime env injected. All it does is put the
# binary in the plugin root — install.sh owns naming, downloading and verification for both paths.
set -eu

ROOT="$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)"
# Cleared for the same reason `kampr update` clears them: a base decides the tarball *and* the
# SHA256SUMS it is checked against, and the bypass decides whether either is looked at. Whatever
# left one of them in this environment — a dotfile, a CI job, a plugin manifest — would otherwise
# be choosing the binary that goes on to type into every terminal in the herd.
exec env -u KAMPR_BASE_URL -u KAMPR_ALLOW_UNVERIFIED \
  KAMPR_PREFIX="$ROOT/bin" KAMPR_MODE=plugin sh "$ROOT/packaging/install.sh"
