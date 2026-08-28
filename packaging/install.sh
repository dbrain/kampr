#!/bin/sh
# Downloads, verifies and installs the kampr binary for this host.
#   curl -fsSL https://github.com/dbrain/kampr/releases/latest/download/install.sh -o install.sh
#   sh install.sh
# Fetch to a file rather than piping into sh: `curl -fsSL ... | sh` pipes nothing into sh on a
# 404 and the pipeline still reports success, so a missing release fails silently.
#
# `fetch-binary.sh` re-enters this script with KAMPR_MODE=plugin, so this is the only place that
# knows how to name, fetch and verify a release. Two copies of that logic would drift.
#
# Environment:
#   KAMPR_PREFIX          where to put the binary (default ~/.local/bin)
#   KAMPR_REPO            owner/repo (default dbrain/kampr)
#   KAMPR_VERSION         tag, or `latest` (default latest)
#   KAMPR_BASE_URL        override the whole download base — a file:// URL works, for testing.
#                         It sources the checksums as well as the tarball, so it is a trust
#                         decision: `kampr update` clears it, and a base you set yourself is
#                         the only one where a missing signature is not fatal
#   KAMPR_MODE            plugin | update | standalone (default standalone)
#                         plugin stops after installing; update also restarts the service but
#                         drops the first-run epilogue, and is what `kampr update` runs
#   KAMPR_REQUIRE_SIGNATURE=1 refuse to install unless cosign is present and the signature checks
#   KAMPR_ALLOW_UNVERIFIED=1  proceed without a checksum, or without a cosign to check the
#                         signature with. Only for a local test build. `kampr update` clears it.
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
# `own_base` is the whole difference between "the operator pointed this at their own directory"
# and "these bytes came from the release the project publishes". A base supplies the tarball *and*
# the SHA256SUMS it is checked against, so on any other base a matching checksum proves nothing.
own_base=
if [ -n "${KAMPR_BASE_URL:-}" ]; then
  base="$KAMPR_BASE_URL"
  own_base=1
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
    https://*)
      # `--proto` governs the protocol of the URL curl is given and says nothing about where a
      # redirect takes it, so a 302 to http is followed without `--proto-redir`. wget is the
      # mirror image: `--https-only` governs the links it follows, not the URL on its command line.
      if command -v curl >/dev/null 2>&1; then
        curl -fsSL --proto '=https' --proto-redir '=https' --tlsv1.2 "$url" -o "$out" && return 0
      elif command -v wget >/dev/null 2>&1; then
        wget -q --https-only -O "$out" "$url" && return 0
      else
        die "neither curl nor wget is available"
      fi ;;
    *) die "refusing to fetch $url over anything but https.
       A release comes from https, or from a file:// base you pointed this at yourself." ;;
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
      # A missing cosign is not evidence of anything, and it used to stop the install dead. That
      # made the *common* case — a host that has never heard of sigstore — indistinguishable from
      # the attack, and the way through it was an environment variable nobody remembers, on every
      # machine, for ever. So the signature is checked when it can be and reported when it cannot.
      #
      # What is *not* softened: a cosign that is present and says no still refuses, because that is
      # evidence. `KAMPR_REQUIRE_SIGNATURE=1` puts the old behaviour back for anyone who wants the
      # install to fail rather than proceed unverified.
      case "$os" in apple-darwin) goos=darwin ;; *) goos=linux ;; esac
      case "$arch" in x86_64) goarch=amd64 ;; *) goarch=arm64 ;; esac
      [ "${KAMPR_REQUIRE_SIGNATURE:-}" != 1 ] || die "cosign is not installed and KAMPR_REQUIRE_SIGNATURE=1 was set,
       so the signature beside SHA256SUMS cannot be checked — refusing to install.

       Install cosign and run this again:
         curl -fsSLo cosign https://github.com/sigstore/cosign/releases/latest/download/cosign-${goos}-${goarch}
         chmod +x cosign && sudo mv cosign /usr/local/bin/cosign
       On macOS, 'brew install cosign'. Other ways, including package managers and a signed
       installer: https://docs.sigstore.dev/cosign/system_config/installation/"
      signature_state="not checked — no cosign on this host; the checksum matched, but a checksum
                    came from the same place as the tarball. Install cosign to check who built it."
    fi
  else
    # An absent signature is the one downgrade an attacker gets for free: serve a tarball, serve
    # checksums that match it, publish no bundle, and the installer says "checksum verified: yes".
    # Every release from this repo is signed by its workflow, so at the canonical base an absent
    # bundle is not an unsigned release — it is not this repo's release.
    [ -n "$own_base" ] || die "no signature alongside SHA256SUMS at ${base} — refusing to install.
       Every kampr release is signed by its release workflow. The checksums that just matched came
       from wherever this tarball did, so without the signature they say nothing about who built it.
       Do not run this file. Report it at https://github.com/${REPO}/issues"
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
[ -w "$PREFIX" ] || die "$PREFIX is not writable — nothing has been changed"

# The binary that works is kept until the new one has proved it runs. The window is one rename
# wide, but on the far side of a bad one is a host with no working kampr and no way to fetch one.
if [ -f "$PREFIX/kampr" ]; then
  cp "$PREFIX/kampr" "$tmp/kampr.previous" || die "could not keep a copy of $PREFIX/kampr — refusing
       to replace a binary that cannot be put back"
fi
# Rename over the old binary rather than writing in place: replacing a running node's file
# in place is ETXTBSY, and a half-written binary is worse than an old one.
mv "$tmp/kampr" "$PREFIX/kampr.new"
mv "$PREFIX/kampr.new" "$PREFIX/kampr"

if ! "$PREFIX/kampr" --version >/dev/null 2>&1; then
  if [ -f "$tmp/kampr.previous" ]; then
    cp "$tmp/kampr.previous" "$PREFIX/kampr.new" && mv "$PREFIX/kampr.new" "$PREFIX/kampr" \
      && die "$asset does not run on this host. The binary you had is back in place, unchanged."
  fi
  die "$PREFIX/kampr does not run on this host"
fi

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

# Upgrading replaces the binary and nothing else: the old one keeps running under its supervisor
# until something restarts it. Only the unit that names *this* binary is ours to restart.
unit="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/kampr.service"
plist="$HOME/Library/LaunchAgents/dev.kampr.node.plist"
if [ -f "$unit" ] && grep -q "^ExecStart=$PREFIX/kampr " "$unit"; then
  if systemctl --user restart kampr.service >/dev/null 2>&1; then
    echo "kampr: restarted kampr.service onto the new binary"
  else
    echo "kampr: run 'systemctl --user restart kampr.service' to move the running node onto it"
  fi
elif [ -f "$plist" ] && grep -q "<string>$PREFIX/kampr</string>" "$plist"; then
  if launchctl kickstart -k "gui/$(id -u)/dev.kampr.node" >/dev/null 2>&1; then
    echo "kampr: restarted dev.kampr.node onto the new binary"
  else
    echo "kampr: run 'launchctl kickstart -k gui/$(id -u)/dev.kampr.node' to move the node onto it"
  fi
fi

# An update is not a first run: the operator already has a paired node and does not need the
# ladder read back to them.
if [ "$MODE" = update ]; then
  exit 0
fi

cat <<'NEXT'

Next:
  kampr init             set up config and print your URL and pairing code
  kampr service install  keep it running across reboots, and say what else that needs
  kampr doctor           check everything that has to be true, and say what is not

It works immediately on your LAN over plain HTTP. Certificates, passkeys,
notifications and extra machines are optional rungs, printed by `kampr status`
and offered from the setup screen in the app on your phone.

To remove it: kampr service uninstall, then delete the binary and the two
directories `kampr status` names.
NEXT
