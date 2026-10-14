#!/usr/bin/env bash
# PreToolUse(Bash). A GRADLE_HOME on this machine silently overrides the wrapper (#67), so a
# gradlew run without `env -u GRADLE_HOME` builds with the wrong Gradle and says nothing.
set -euo pipefail
command=$(jq -r '.tool_input.command // empty')
bare=$(printf '%s\n' "$command" \
  | sed -E 's/(&&|\|\||;|\|)/\n/g' \
  | grep -E '(^|[[:space:]/])gradlew([[:space:]]|$)' \
  | grep -vE -- '-u[[:space:]]+GRADLE_HOME([[:space:]]|$)' || true)
if [ -n "$bare" ]; then
  echo "Blocked: run gradlew as \`env -u GRADLE_HOME ./gradlew …\` from client/, or use a make target. A GRADLE_HOME here silently overrides the wrapper (docs/03-probe-log.md #67)." >&2
  exit 2
fi
