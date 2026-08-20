#!/bin/sh
# Downloads the kampr binary matching this host into the plugin root.
# Runs at `herdr plugin install` time, with no plugin runtime env injected.
set -eu

REPO="${KAMPR_REPO:-dbrain/kampr}"
VERSION="${KAMPR_VERSION:-latest}"
ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"

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

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "kampr: fetching ${asset}"
if command -v curl >/dev/null 2>&1; then
  curl -fsSL "$url" -o "$tmp/$asset"
elif command -v wget >/dev/null 2>&1; then
  wget -qO "$tmp/$asset" "$url"
else
  echo "kampr: neither curl nor wget is available" >&2; exit 1
fi

tar -xzf "$tmp/$asset" -C "$tmp"
mkdir -p "$ROOT/bin"
mv "$tmp/kampr" "$ROOT/bin/kampr"
chmod +x "$ROOT/bin/kampr"
"$ROOT/bin/kampr" --version
