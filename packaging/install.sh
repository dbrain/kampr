#!/bin/sh
# Standalone install, for people not using the Herdr plugin surface.
#   curl -fsSL https://kampr.dev/install.sh | sh
set -eu

PREFIX="${KAMPR_PREFIX:-$HOME/.local/bin}"
REPO="${KAMPR_REPO:-dbrain/kampr}"
VERSION="${KAMPR_VERSION:-latest}"

case "$(uname -s)" in
  Linux)  os=unknown-linux-gnu ;;
  Darwin) os=apple-darwin ;;
  *) echo "kampr: unsupported OS $(uname -s)" >&2; exit 1 ;;
esac
case "$(uname -m)" in
  x86_64|amd64) arch=x86_64 ;;
  arm64|aarch64) arch=aarch64 ;;
  *) echo "kampr: unsupported architecture $(uname -m)" >&2; exit 1 ;;
esac

asset="kampr-${arch}-${os}.tar.gz"
if [ "$VERSION" = latest ]; then
  url="https://github.com/${REPO}/releases/latest/download/${asset}"
else
  url="https://github.com/${REPO}/releases/download/${VERSION}/${asset}"
fi

tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
echo "kampr: fetching ${asset}"
curl -fsSL "$url" -o "$tmp/$asset"
tar -xzf "$tmp/$asset" -C "$tmp"
mkdir -p "$PREFIX"
mv "$tmp/kampr" "$PREFIX/kampr"
chmod +x "$PREFIX/kampr"

echo
"$PREFIX/kampr" --version
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
