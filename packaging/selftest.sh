#!/bin/sh
# Exercises install.sh against a fabricated local release. No network, no real binary.
# Run it from anywhere:  sh packaging/selftest.sh
set -eu

HERE="$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)"
ROOT="$(dirname "$HERE")"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
fails=0

case "$(uname -s)" in Linux) os=unknown-linux-musl ;; Darwin) os=apple-darwin ;; *) echo "selftest: unsupported host"; exit 0 ;; esac
case "$(uname -m)" in x86_64|amd64) arch=x86_64 ;; arm64|aarch64) arch=aarch64 ;; *) echo "selftest: unsupported host"; exit 0 ;; esac
asset="kampr-${arch}-${os}.tar.gz"

sum() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$@"; else shasum -a 256 "$@"; fi
}

release() {
  rm -rf "$work/rel" "$work/stage"
  mkdir -p "$work/rel" "$work/stage"
  printf '#!/bin/sh\necho "kampr 0.0.0-selftest"\n' > "$work/stage/kampr"
  chmod +x "$work/stage/kampr"
  tar -czf "$work/rel/$asset" -C "$work/stage" kampr
  (cd "$work/rel" && sum "$asset" > SHA256SUMS)
}

run() {
  rm -rf "$work/prefix"
  KAMPR_BASE_URL="file://$work/rel" KAMPR_PREFIX="$work/prefix" \
    sh "$ROOT/packaging/install.sh" > "$work/out" 2>&1
}

check() {
  name="$1"; expect="$2"; pattern="$3"
  if run; then got=pass; else got=fail; fi
  if [ "$got" != "$expect" ]; then
    echo "FAIL $name: expected $expect, got $got"; sed 's/^/     | /' "$work/out"; fails=$((fails + 1)); return
  fi
  if ! grep -q "$pattern" "$work/out"; then
    echo "FAIL $name: output did not contain '$pattern'"; sed 's/^/     | /' "$work/out"; fails=$((fails + 1)); return
  fi
  echo "ok   $name"
}

release
check "a good release installs" pass "checksum verified: yes"
[ -x "$work/prefix/kampr" ] || { echo "FAIL: no binary at the prefix"; fails=$((fails + 1)); }

release
check "no signature is reported, not hidden" pass "signature verified: skipped — this release publishes no signature"

release
: > "$work/rel/SHA256SUMS.cosign.bundle"
if command -v cosign >/dev/null 2>&1; then
  check "a bogus signature is refused" fail "did not verify"
else
  # A host with no cosign installs and *says* the signature went unchecked. Softening this is what
  # stopped the common case — a machine that has never heard of sigstore — being indistinguishable
  # from an attack; the refusal moved to KAMPR_REQUIRE_SIGNATURE=1, which is the next case.
  check "a missing cosign is reported, not skipped silently" pass "signature verified: not checked — no cosign on this host"
fi

# **The guard that replaced the old refusal, and it was covered by nothing.** Making a missing
# cosign non-fatal took a check away from every install; `KAMPR_REQUIRE_SIGNATURE=1` is where it
# went, so an operator who wants the old behaviour has somewhere to get it. A softening whose
# escape hatch is untested has quietly removed the guard rather than moved it.
release
: > "$work/rel/SHA256SUMS.cosign.bundle"
if ! command -v cosign >/dev/null 2>&1; then
  rm -rf "$work/prefix"
  if KAMPR_REQUIRE_SIGNATURE=1 KAMPR_BASE_URL="file://$work/rel" KAMPR_PREFIX="$work/prefix" \
       sh "$ROOT/packaging/install.sh" > "$work/out" 2>&1; then
    echo "FAIL asking for the signature to be required installed anyway"; sed 's/^/     | /' "$work/out"; fails=$((fails + 1))
  elif ! grep -q "KAMPR_REQUIRE_SIGNATURE=1 was set" "$work/out"; then
    echo "FAIL the refusal did not say which setting caused it"; sed 's/^/     | /' "$work/out"; fails=$((fails + 1))
  elif [ -e "$work/prefix/kampr" ]; then
    echo "FAIL it refused and installed the binary anyway"; fails=$((fails + 1))
  else
    echo "ok   requiring the signature still refuses a host with no cosign"
  fi
fi

release
sed -i.bak 's/^[0-9a-f]/z/' "$work/rel/SHA256SUMS" 2>/dev/null || sed 's/^[0-9a-f]/z/' "$work/rel/SHA256SUMS.bak" > "$work/rel/SHA256SUMS"
check "a tampered checksum is refused" fail "checksum mismatch"

release
rm -f "$work/rel/SHA256SUMS"
check "a release with no checksums is refused" fail "refusing to install"

release
rm -f "$work/rel/SHA256SUMS"
rm -rf "$work/prefix"
if KAMPR_ALLOW_UNVERIFIED=1 KAMPR_BASE_URL="file://$work/rel" KAMPR_PREFIX="$work/prefix" \
     sh "$ROOT/packaging/install.sh" > "$work/out" 2>&1 &&
   grep -q "checksum verified: skipped" "$work/out"; then
  echo "ok   the escape hatch is explicit about what it skipped"
else
  echo "FAIL the escape hatch did not report itself"; sed 's/^/     | /' "$work/out"; fails=$((fails + 1))
fi

release
sed -i.bak "s|$asset|kampr-somethingelse.tar.gz|" "$work/rel/SHA256SUMS" 2>/dev/null ||
  sed "s|$asset|kampr-somethingelse.tar.gz|" "$work/rel/SHA256SUMS.bak" > "$work/rel/SHA256SUMS"
check "an unlisted asset is refused" fail "does not list"

# The plugin path is the same code with a different prefix, and this is the assertion that keeps
# `herdr plugin install` honest: the binary must land where kamprctl.sh looks for it.
#
# Driven through install.sh rather than through fetch-binary.sh, because fetch-binary.sh now drops
# KAMPR_BASE_URL on the way in and a file:// base cannot reach it. That it drops them is the thing
# under test in `the_plugin_install_path_does_not_inherit_the_bypasses`, which has a curl on PATH
# to answer for the canonical base and so can drive the real entry point offline.
release
rm -rf "$work/plugin"
mkdir -p "$work/plugin/bin"
if KAMPR_BASE_URL="file://$work/rel" KAMPR_PREFIX="$work/plugin/bin" KAMPR_MODE=plugin \
     sh "$ROOT/packaging/install.sh" > "$work/out" 2>&1 &&
   [ -x "$work/plugin/bin/kampr" ] && ! grep -q "kampr init" "$work/out"; then
  echo "ok   the plugin prefix gets the binary, without the standalone epilogue"
else
  echo "FAIL the plugin install path"; sed 's/^/     | /' "$work/out"; fails=$((fails + 1))
fi

if [ "$fails" -gt 0 ]; then
  echo "$fails failure(s)"; exit 1
fi
echo "all install-path checks passed"
