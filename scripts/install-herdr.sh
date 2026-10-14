#!/bin/sh
# CI only: the live suites fail outright without a herdr, and a bare runner never has one.
set -eu
curl -fsSL https://herdr.dev/latest.json -o latest.json
url=$(jq -er '.assets["linux-x86_64"]' latest.json)
want=$(jq -er '.sha256["linux-x86_64"]' latest.json)
curl -fsSL "$url" -o herdr
echo "$want  herdr" | sha256sum -c -
chmod +x herdr
mkdir -p "$HOME/.local/bin"
mv herdr "$HOME/.local/bin/herdr"
rm latest.json
if [ -n "${GITHUB_PATH:-}" ]; then echo "$HOME/.local/bin" >> "$GITHUB_PATH"; fi
