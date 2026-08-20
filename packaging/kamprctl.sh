#!/bin/sh
# Plugin action dispatcher. Herdr injects HERDR_SOCKET_PATH, HERDR_BIN_PATH, HERDR_PLUGIN_ROOT,
# HERDR_PLUGIN_CONFIG_DIR and HERDR_PLUGIN_STATE_DIR; a service started from here outlives Herdr,
# so it must not depend on any of them still being set later.
set -eu

ROOT="${HERDR_PLUGIN_ROOT:-$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)}"
CONFIG_DIR="${HERDR_PLUGIN_CONFIG_DIR:-${XDG_CONFIG_HOME:-$HOME/.config}/kampr}"
STATE_DIR="${HERDR_PLUGIN_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/kampr}"
BIN="$ROOT/bin/kampr"
[ -x "$BIN" ] || BIN="$(command -v kampr || echo "$ROOT/bin/kampr")"
UNIT=kampr.service
LABEL=dev.kampr.node

case "$(uname -s)" in
  Darwin) SUPERVISOR=launchd ;;
  Linux)  SUPERVISOR=systemd ;;
  *) echo "kampr: $(uname -s) is not a supported host — the node needs a Unix socket to reach Herdr" >&2
     exit 1 ;;
esac

UID_="$(id -u)"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
DOMAIN="gui/$UID_"

launchd_loaded() { launchctl print "$DOMAIN/$LABEL" >/dev/null 2>&1; }

svc() {
  case "$SUPERVISOR" in
    systemd) systemctl --user "$@" "$UNIT" ;;
    launchd)
      case "$1" in
        start)
          if launchd_loaded; then
            launchctl kickstart "$DOMAIN/$LABEL"
          else
            launchctl bootstrap "$DOMAIN" "$PLIST"
          fi ;;
        stop)    launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true ;;
        restart)
          if launchd_loaded; then
            launchctl kickstart -k "$DOMAIN/$LABEL"
          else
            launchctl bootstrap "$DOMAIN" "$PLIST"
          fi ;;
        status)  launchctl print "$DOMAIN/$LABEL" 2>/dev/null || echo "kampr: not loaded" ;;
      esac ;;
  esac
}

render() {
  sed -e "s|@BIN@|$BIN|g" -e "s|@CONFIG_DIR@|$CONFIG_DIR|g" -e "s|@STATE_DIR@|$STATE_DIR|g" \
      -e "s|@SOCKET@|${HERDR_SOCKET_PATH:-$2}|g" "$1"
}

# Rendering is idempotent, but reloading is not: a bootout/bootstrap cycle kills a healthy node.
# So the unit is only reloaded when its content actually changed.
install_unit() {
  mkdir -p "$CONFIG_DIR" "$STATE_DIR"
  case "$SUPERVISOR" in
    systemd)
      dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
      mkdir -p "$dir"
      render "$ROOT/packaging/kampr.service" "%h/.config/herdr/herdr.sock" > "$dir/$UNIT"
      # The unit file is the durable artefact; a systemd that cannot be reached — a container, a
      # session-less login — must not cost us the write. `svc start` reports the real failure.
      systemctl --user daemon-reload >/dev/null 2>&1 || true
      systemctl --user enable "$UNIT" >/dev/null 2>&1 || true
      ;;
    launchd)
      # launchd does no %h expansion, so the socket default has to be a real path.
      mkdir -p "$(dirname "$PLIST")"
      tmp="$PLIST.new"
      render "$ROOT/packaging/$LABEL.plist" "$HOME/.config/herdr/herdr.sock" > "$tmp"
      if [ -f "$PLIST" ] && cmp -s "$tmp" "$PLIST" && launchd_loaded; then
        rm -f "$tmp"
        return 0
      fi
      mv "$tmp" "$PLIST"
      launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
      launchctl bootstrap "$DOMAIN" "$PLIST"
      ;;
  esac
}

remove_unit() {
  case "$SUPERVISOR" in
    systemd)
      systemctl --user disable "$UNIT" >/dev/null 2>&1 || true
      rm -f "${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/$UNIT"
      systemctl --user daemon-reload >/dev/null 2>&1 || true
      ;;
    launchd) rm -f "$PLIST" ;;
  esac
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
  stop)    svc stop || true ;;
  restart) require_binary; install_unit; svc restart ;;
  status)  require_binary; "$BIN" status --config-dir "$CONFIG_DIR" || true; svc status || true ;;
  url)     require_binary; exec "$BIN" url --config-dir "$CONFIG_DIR" ;;
  update)
    sh "$ROOT/packaging/fetch-binary.sh"
    install_unit
    svc restart
    ;;
  uninstall)
    svc stop || true
    remove_unit
    echo "kampr: service removed. Devices and config are kept at:"
    echo "         $CONFIG_DIR"
    echo "         $STATE_DIR"
    echo "       To delete them too — every paired device loses access — run:"
    echo "         sh $ROOT/packaging/kamprctl.sh purge"
    ;;
  # Deliberately not a Herdr action: it revokes every paired device, and an action list is one tap
  # away from a phone. Reaching it takes a shell, which is the confirmation.
  purge)
    svc stop || true
    remove_unit
    rm -rf "$CONFIG_DIR" "$STATE_DIR"
    echo "kampr: service, units, devices and config removed."
    ;;
  *) echo "usage: kamprctl.sh {setup|nudge|start|stop|restart|status|url|update|uninstall|purge}" >&2; exit 2 ;;
esac
