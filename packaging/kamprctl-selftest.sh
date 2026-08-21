#!/bin/sh
# Exercises kamprctl.sh — the plugin's only entry point — against a fake kampr binary and a
# throwaway HOME. Nothing here touches the caller's systemd, launchd, config or state.
#   sh packaging/kamprctl-selftest.sh
set -eu

HERE="$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)"
ROOT="$(dirname "$HERE")"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
fails=0

case "$(uname -s)" in
  Linux)  if [ -d /run/systemd/system ]; then supervisor=systemd; else supervisor=none; fi ;;
  Darwin) supervisor=launchd ;;
  *) echo "kamprctl-selftest: unsupported host"; exit 0 ;;
esac

# A stand-in for the real binary: it records every argv it is handed, and `init` is the only
# subcommand with a side effect — the config file whose absence is the whole bug.
plugin="$work/plugin"
mkdir -p "$plugin/bin" "$plugin/packaging"
cp "$ROOT/packaging/kampr.service" "$ROOT/packaging/dev.kampr.node.plist" \
   "$ROOT/packaging/kamprctl.sh" "$plugin/packaging/"
cat > "$plugin/bin/kampr" <<'FAKE'
#!/bin/sh
printf '%s\n' "$*" >> "$ARGLOG"
sub="$1"; shift
cfg=""
while [ $# -gt 0 ]; do
  if [ "$1" = --config-dir ]; then cfg="$2"; shift 2; else shift; fi
done
if [ "$sub" = init ] && [ -n "$cfg" ]; then
  mkdir -p "$cfg"
  printf 'node_id = "fake"\n' > "$cfg/config.toml"
fi
exit 0
FAKE
chmod +x "$plugin/bin/kampr"

reset() {
  rm -rf "${work:?}/home" "${work:?}/cfg" "${work:?}/state" "${work:?}/args"
  mkdir -p "$work/home"
  : > "$work/args"
}

ctl() {
  HOME="$work/home" \
  XDG_CONFIG_HOME="$work/home/.config" \
  XDG_STATE_HOME="$work/home/.local/state" \
  XDG_RUNTIME_DIR="$work/nowhere/run" \
  DBUS_SESSION_BUS_ADDRESS="unix:path=$work/nowhere/bus" \
  DBUS_SYSTEM_BUS_ADDRESS="unix:path=$work/nowhere/bus" \
  HERDR_PLUGIN_ROOT="$plugin" \
  HERDR_PLUGIN_CONFIG_DIR="$work/cfg" \
  HERDR_PLUGIN_STATE_DIR="$work/state" \
  ARGLOG="$work/args" \
    sh "$plugin/packaging/kamprctl.sh" "$@" > "$work/out" 2> "$work/err"
}

fail() {
  echo "FAIL $1"
  shift
  [ $# -eq 0 ] || echo "     $*"
  sed 's/^/     out| /' "$work/out"
  sed 's/^/     err| /' "$work/err"
  fails=$((fails + 1))
}

unit_path="$work/home/.config/systemd/user/kampr.service"
plist_path="$work/home/Library/LaunchAgents/dev.kampr.node.plist"
installed_unit() {
  case "$supervisor" in
    systemd) [ -f "$unit_path" ] ;;
    launchd) [ -f "$plist_path" ] ;;
    none)    return 1 ;;
  esac
}

# 1 — the whole of B2: the plugin's only entry point never ran `kampr init`, so every command
#     behind it died on a config nothing created.
reset
if ctl setup; then
  if head -n 1 "$work/args" | grep -q '^init '; then
    if grep -q '^setup ' "$work/args"; then
      echo "ok   setup initialises before it sets up"
    else
      fail "setup runs the setup ladder" "argv log: $(cat "$work/args")"
    fi
  else
    fail "setup runs init first" "argv log: $(cat "$work/args")"
  fi
else
  fail "setup succeeds on a fresh config directory"
fi

# 2 — init is once. A second setup must not re-run it over a live node.
: > "$work/args"
if ctl setup; then
  if grep -q '^init ' "$work/args"; then
    fail "setup does not re-init an existing node" "argv log: $(cat "$work/args")"
  else
    echo "ok   setup leaves an initialised node alone"
  fi
else
  fail "a second setup succeeds"
fi

# 3 — the startup nudge against an uninitialised node. `kampr serve` exits 1 without a config and
#     the unit restarts on failure with no limit, so starting one is a 5-second loop forever.
reset
if ctl nudge; then
  if installed_unit; then
    fail "nudge does not arm a restart loop against a node that cannot start"
  elif [ -s "$work/args" ]; then
    fail "nudge runs nothing without a config" "argv log: $(cat "$work/args")"
  else
    echo "ok   nudge is a no-op on an uninitialised node"
  fi
else
  fail "nudge exits zero on an uninitialised node"
fi

# 4 — and the same for the actions a person can tap.
for action in start restart; do
  reset
  if ctl "$action"; then
    fail "$action refuses an uninitialised node"
  elif grep -q 'kamprctl.sh setup' "$work/err"; then
    echo "ok   $action says how to initialise instead of looping"
  else
    fail "$action says how to initialise"
  fi
done

# 5 — B1 on the plugin path: a user unit that nothing lingers for dies at logout and is not
#     started at boot, and nothing in the tree said so.
lingers() {
  user="$(id -un)"
  if [ -d /var/lib/systemd/linger ]; then
    [ -e "/var/lib/systemd/linger/$user" ]
  else
    [ "$(loginctl show-user "$user" -p Linger --value 2>/dev/null || echo no)" = yes ]
  fi
}
if [ "$supervisor" = systemd ] && ! lingers; then
  reset
  ctl setup || true
  if grep -q 'loginctl enable-linger' "$work/err" "$work/out"; then
    echo "ok   installing the unit names the linger requirement"
  else
    fail "installing the unit names the linger requirement"
  fi
else
  echo "skip linger (not a systemd host, or this user already lingers)"
fi

# 6 — S10: the unit's ExecStart may not point inside the plugin checkout, because Herdr replaces
#     that checkout wholesale on reinstall and the node is running out of it.
if [ "$supervisor" = systemd ]; then
  reset
  ctl setup || true
  if [ -f "$unit_path" ]; then
    if grep -q "^ExecStart=$plugin/bin/kampr " "$unit_path"; then
      fail "the unit points outside the managed checkout" "$(grep ExecStart "$unit_path")"
    else
      echo "ok   the unit points at a binary a plugin reinstall cannot replace"
    fi
  else
    fail "setup installs a unit"
  fi
else
  echo "skip ExecStart location (no systemd on this host)"
fi

if [ "$fails" -gt 0 ]; then
  echo
  echo "$fails check(s) failed"
  exit 1
fi
echo
echo "kamprctl: all checks passed"
