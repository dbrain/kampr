#!/bin/sh
# Plugin action dispatcher. Herdr injects HERDR_SOCKET_PATH, HERDR_BIN_PATH, HERDR_PLUGIN_ROOT,
# HERDR_PLUGIN_CONFIG_DIR and HERDR_PLUGIN_STATE_DIR; a service started from here outlives Herdr,
# so it must not depend on any of them still being set later.
set -eu

ROOT="${HERDR_PLUGIN_ROOT:-$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)}"
CONFIG_DIR="${HERDR_PLUGIN_CONFIG_DIR:-${XDG_CONFIG_HOME:-$HOME/.config}/kampr}"
STATE_DIR="${HERDR_PLUGIN_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/kampr}"
BIN="$ROOT/bin/kampr"
[ -x "$BIN" ] || BIN="$(command -v kampr || echo "$ROOT/bin/kampr")"
UNIT=kampr.service

case "$(uname -s)" in
  Darwin) SUPERVISOR=launchd ;;
  *)      SUPERVISOR=systemd ;;
esac

svc() {
  case "$SUPERVISOR" in
    systemd) systemctl --user "$@" "$UNIT" ;;
    launchd)
      label=dev.kampr.node
      case "$1" in
        start)   launchctl kickstart -k "gui/$(id -u)/$label" ;;
        stop)    launchctl bootout "gui/$(id -u)/$label" 2>/dev/null || true ;;
        restart) launchctl kickstart -k "gui/$(id -u)/$label" ;;
        status)  launchctl print "gui/$(id -u)/$label" 2>/dev/null || echo "kampr: not loaded" ;;
        *) : ;;
      esac ;;
  esac
}

install_unit() {
  [ "$SUPERVISOR" = systemd ] || return 0
  dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
  mkdir -p "$dir" "$CONFIG_DIR" "$STATE_DIR"
  sed -e "s|@BIN@|$BIN|g" -e "s|@CONFIG_DIR@|$CONFIG_DIR|g" -e "s|@STATE_DIR@|$STATE_DIR|g" \
      -e "s|@SOCKET@|${HERDR_SOCKET_PATH:-%h/.config/herdr/herdr.sock}|g" \
      "$ROOT/packaging/kampr.service" > "$dir/$UNIT"
  systemctl --user daemon-reload
  systemctl --user enable "$UNIT" >/dev/null 2>&1 || true
}

require_binary() {
  [ -x "$BIN" ] && return 0
  echo "kampr: binary not found at $BIN" >&2
  echo "       run: sh $ROOT/packaging/fetch-binary.sh" >&2
  exit 1
}

case "${1:-status}" in
  setup)
    require_binary
    install_unit
    svc start
    exec "$BIN" setup --config-dir "$CONFIG_DIR" --state-dir "$STATE_DIR"
    ;;
  # Startup hooks are one-shot, not supervised, so this only ensures the unit exists and is running.
  # A healthy node makes it a no-op.
  nudge)
    [ -x "$BIN" ] || exit 0
    install_unit
    svc start >/dev/null 2>&1 || true
    ;;
  start)   require_binary; install_unit; svc start ;;
  stop)    svc stop ;;
  restart) require_binary; install_unit; svc restart ;;
  status)  require_binary; "$BIN" status --config-dir "$CONFIG_DIR" || true; svc status ;;
  url)     require_binary; exec "$BIN" url --config-dir "$CONFIG_DIR" ;;
  update)
    sh "$ROOT/packaging/fetch-binary.sh"
    install_unit
    svc restart
    ;;
  uninstall)
    svc stop
    if [ "$SUPERVISOR" = systemd ]; then
      systemctl --user disable "$UNIT" >/dev/null 2>&1 || true
      rm -f "${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/$UNIT"
      systemctl --user daemon-reload
    fi
    echo "kampr: service removed. Config kept at $CONFIG_DIR; delete it by hand to reset."
    ;;
  *) echo "usage: kamprctl.sh {setup|nudge|start|stop|restart|status|url|update|uninstall}" >&2; exit 2 ;;
esac
