#!/bin/sh
# Downloads, verifies and installs the kampr binary for this host.
#   curl -fsSL https://kampr.dev/install.sh | sh
#
# `fetch-binary.sh` re-enters this script with KAMPR_MODE=plugin, so this is the only place that
# knows how to name, fetch and verify a release. Two copies of that logic would drift.
#
# Environment:
#   KAMPR_PREFIX          where to put the binary (default ~/.local/bin)
#   KAMPR_REPO            owner/repo (default dbrain/kampr)
#   KAMPR_VERSION         tag, or `latest` (default latest)
#   KAMPR_BASE_URL        override the whole download base — a file:// URL works, for testing
#   KAMPR_MODE            plugin | standalone (default standalone; plugin drops the epilogue)
#   KAMPR_ALLOW_UNVERIFIED=1  proceed without a checksum. Only for a local test build.
set -eu

PREFIX="${KAMPR_PREFIX:-$HOME/.local/bin}"
REPO="${KAMPR_REPO:-dbrain/kampr}"
VERSION="${KAMPR_VERSION:-latest}"
MODE="${KAMPR_MODE:-standalone}"
ISSUER="https://token.actions.githubusercontent.com"

die() { echo "kampr: $*" >&2; exit 1; }

case "$(uname -s)" in
  Linux)  os=unknown-linux-musl ;;
  Darwin) os=apple-darwin ;;
  MINGW*|MSYS*|CYGWIN*|Windows_NT)
    die "Windows is not supported. The node reaches Herdr over a Unix domain socket and
       supervises itself with systemd or launchd; neither exists on Windows. Use WSL2." ;;
  *) die "unsupported OS $(uname -s)" ;;
esac
case "$(uname -m)" in
  x86_64|amd64) arch=x86_64 ;;
  arm64|aarch64) arch=aarch64 ;;
  *) die "unsupported architecture $(uname -m)" ;;
esac

asset="kampr-${arch}-${os}.tar.gz"
if [ -n "${KAMPR_BASE_URL:-}" ]; then
  base="$KAMPR_BASE_URL"
elif [ "$VERSION" = latest ]; then
  base="https://github.com/${REPO}/releases/latest/download"
else
  base="https://github.com/${REPO}/releases/download/${VERSION}"
fi

# `optional` distinguishes "the signature bundle is absent" from "the tarball is absent".
fetch() {
  url="$1"; out="$2"; optional="${3:-}"
  case "$url" in
    file://*) cp "${url#file://}" "$out" 2>/dev/null && return 0 ;;
    *)
      if command -v curl >/dev/null 2>&1; then
        curl -fsSL --proto '=https' --tlsv1.2 "$url" -o "$out" && return 0
      elif command -v wget >/dev/null 2>&1; then
        wget -q --https-only -O "$out" "$url" && return 0
      else
        die "neither curl nor wget is available"
      fi ;;
  esac
  rm -f "$out"
  [ -n "$optional" ] && return 1
  die "could not download $url"
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  elif command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | cut -d' ' -f1
  elif command -v openssl >/dev/null 2>&1; then openssl dgst -sha256 "$1" | sed 's/.*= //'
  else return 1
  fi
}

tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT

echo "kampr: fetching ${asset}"
fetch "${base}/${asset}" "$tmp/$asset"

# --- verification ------------------------------------------------------------------------------
checksum_state=no
signature_state="not attempted"

if fetch "${base}/SHA256SUMS" "$tmp/SHA256SUMS" optional; then
  expected="$(awk -v n="$asset" '{ sub(/^\*/, "", $2); if ($2 == n) print $1 }' "$tmp/SHA256SUMS")"
  [ -n "$expected" ] || die "SHA256SUMS does not list $asset — refusing to install"
  if actual="$(sha256_of "$tmp/$asset")"; then
    [ "$actual" = "$expected" ] || die "checksum mismatch for $asset
       expected $expected
       got      $actual
       Do not run this file. Report it at https://github.com/${REPO}/issues"
    checksum_state=yes
  else
    [ "${KAMPR_ALLOW_UNVERIFIED:-}" = 1 ] || die "no sha256 tool found (sha256sum, shasum or openssl).
       Install one, or re-run with KAMPR_ALLOW_UNVERIFIED=1 to accept an unverified binary."
    checksum_state="skipped — no sha256 tool on this host"
  fi

  if fetch "${base}/SHA256SUMS.cosign.bundle" "$tmp/SHA256SUMS.cosign.bundle" optional; then
    if command -v cosign >/dev/null 2>&1; then
      if cosign verify-blob \
           --bundle "$tmp/SHA256SUMS.cosign.bundle" \
           --certificate-oidc-issuer "$ISSUER" \
           --certificate-identity-regexp "^https://github\.com/${REPO}/\.github/workflows/release\.yml@refs/tags/" \
           "$tmp/SHA256SUMS" >/dev/null 2>&1
      then
        signature_state="yes — signed by ${REPO}'s release workflow"
      else
        die "the signature on SHA256SUMS did not verify against ${REPO}'s release workflow.
       Do not run this file. Report it at https://github.com/${REPO}/issues"
      fi
    else
      signature_state="skipped — cosign is not installed (https://docs.sigstore.dev/cosign/installation)"
    fi
  else
    signature_state="skipped — this release publishes no signature"
  fi
else
  [ "${KAMPR_ALLOW_UNVERIFIED:-}" = 1 ] || die "no SHA256SUMS alongside $asset — refusing to install
       an unverified binary that would be given access to every terminal in your herd.
       Set KAMPR_ALLOW_UNVERIFIED=1 only if you produced this build yourself."
  checksum_state="skipped — release published no SHA256SUMS"
fi

# --- install -----------------------------------------------------------------------------------
tar -xzf "$tmp/$asset" -C "$tmp"
[ -f "$tmp/kampr" ] || die "$asset does not contain a kampr binary"
chmod +x "$tmp/kampr"
mkdir -p "$PREFIX"
# Rename over the old binary rather than writing in place: replacing a running node's file
# in place is ETXTBSY, and a half-written binary is worse than an old one.
mv "$tmp/kampr" "$PREFIX/kampr.new"
mv "$PREFIX/kampr.new" "$PREFIX/kampr"

"$PREFIX/kampr" --version >/dev/null 2>&1 || die "$PREFIX/kampr does not run on this host"

echo "kampr: installed $("$PREFIX/kampr" --version) to $PREFIX/kampr"
echo "kampr: checksum verified: $checksum_state"
echo "kampr: signature verified: $signature_state"

if [ "$MODE" = plugin ]; then
  exit 0
fi

case ":$PATH:" in
  *":$PREFIX:"*) ;;
  *) echo "kampr: $PREFIX is not on your PATH — add it, or run $PREFIX/kampr directly" ;;
esac
cat <<'NEXT'

Next:
  kampr init             set up config and print your URL and pairing code
  kampr service install  keep it running across reboots

It works immediately on your LAN over plain HTTP. Certificates, passkeys,
notifications and extra machines are optional and offered from the setup screen.
NEXT
