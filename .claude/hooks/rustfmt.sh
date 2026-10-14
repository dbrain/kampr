#!/usr/bin/env bash
# PostToolUse(Edit|Write). Formats the Rust file just written, so `make fmt` never fails on an
# agent's edit. A file that does not parse yet is left alone: the edit after it will.
set -euo pipefail
file=$(jq -r '.tool_input.file_path // empty')
root=${CLAUDE_PROJECT_DIR:-$(pwd)}
case "$file" in
  "$root"/*.rs)
    if [ -f "$file" ]; then
      rustfmt --edition 2024 "$file" 2>/dev/null || true
    fi
    ;;
esac
