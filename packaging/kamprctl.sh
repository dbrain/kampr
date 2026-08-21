#!/bin/sh
# Plugin action dispatcher. Herdr injects HERDR_SOCKET_PATH, HERDR_BIN_PATH, HERDR_PLUGIN_ROOT,
# HERDR_PLUGIN_CONFIG_DIR and HERDR_PLUGIN_STATE_DIR; a service started from here outlives Herdr,
# so it must not depend on any of them still being set later.
set -eu

ROOT="${HERDR_PLUGIN_ROOT:-$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)}"
CONFIG_DIR="${HERDR_PLUGIN_CONFIG_DIR:-${XDG_CONFIG_HOME:-$HOME/.config}/kampr}"
STATE_DIR="${HERDR_PLUGIN_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/kampr}"
# A GitHub plugin root is a managed checkout that Herdr replaces wholesale on reinstall, so the
# binary the unit's ExecStart names cannot live in it: a refresh would swap it out from under a
# running node. `fetch-binary.sh` still lands it there, and this stages it somewhere durable.
SOURCE="$ROOT/bin/kampr"
[ -x "$SOURCE" ] || SOURCE="$(command -v kampr || echo "$ROOT/bin/kampr")"
BIN="$STATE_DIR/bin/kampr"
UNIT=kampr.service
LABEL=dev.kampr.node

case "$(uname -s)" in
  Darwin) SUPERVISOR=launchd ;;
  Linux)
    # sd_booted(3). WSL2 without `systemd=true`, OpenRC and a plain container all fail this, and
    # a user unit written on one of those is a file nothing will ever read.
    if [ -d /run/systemd/system ]; then SUPERVISOR=systemd; else SUPERVISOR=none; fi ;;
  *) echo "kampr: $(uname -s) is not a supported host — the node needs a Unix socket to reach Herdr" >&2
     exit 1 ;;
esac

UID_="$(id -u)"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
DOMAIN="gui/$UID_"

launchd_loaded() { launchctl print "$DOMAIN/$LABEL" >/dev/null 2>&1; }

svc() {
  case "$SUPERVISOR" in
    none) return 0 ;;
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
    none)
      echo "kampr: this host has no systemd, so there is no user unit to install." >&2
      echo "       WSL2 needs 'systemd=true' in /etc/wsl.conf and a 'wsl --shutdown'." >&2
      return 0 ;;
    systemd)
      dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
      mkdir -p "$dir"
      render "$ROOT/packaging/kampr.service" "%h/.config/herdr/herdr.sock" > "$dir/$UNIT"
      # The unit file is the durable artefact; a systemd that cannot be reached — a container, a
      # session-less login — must not cost us the write. `svc start` reports the real failure.
      systemctl --user daemon-reload >/dev/null 2>&1 || true
      systemctl --user enable "$UNIT" >/dev/null 2>&1 || true
      enable_linger
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

# `enable` is not "survives a reboot": a systemd --user manager lives inside the caller's login
# session and is not started at boot unless the user lingers.
lingers() {
  user="$(id -un)"
  if [ -d /var/lib/systemd/linger ]; then
    [ -e "/var/lib/systemd/linger/$user" ]
  else
    [ "$(loginctl show-user "$user" -p Linger --value 2>/dev/null || echo no)" = yes ]
  fi
}

enable_linger() {
  if lingers; then return 0; fi
  loginctl enable-linger "$(id -un)" >/dev/null 2>&1 || true
  if lingers; then return 0; fi
  echo "kampr: REQUIRED — without this the node does not come back after a reboot:" >&2
  echo "         loginctl enable-linger $(id -un)" >&2
  echo "       systemd tears your user manager down when your last session ends and does not" >&2
  echo "       start it at boot, so $UNIT stops at logout and stays stopped." >&2
}

remove_unit() {
  case "$SUPERVISOR" in
    none) return 0 ;;
    systemd)
      systemctl --user disable "$UNIT" >/dev/null 2>&1 || true
      rm -f "${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/$UNIT"
      systemctl --user daemon-reload >/dev/null 2>&1 || true
      ;;
    launchd) rm -f "$PLIST" ;;
  esac
}

# Renamed into place, never written in place: replacing a running node's own file is ETXTBSY.
stage_binary() {
  if [ ! -x "$SOURCE" ]; then return 0; fi
  if [ -x "$BIN" ] && cmp -s "$SOURCE" "$BIN"; then return 0; fi
  mkdir -p "$STATE_DIR/bin"
  cp "$SOURCE" "$BIN.new"
  chmod +x "$BIN.new"
  mv "$BIN.new" "$BIN"
}

require_binary() {
  stage_binary
  if [ -x "$BIN" ]; then return 0; fi
  echo "kampr: binary not found at $SOURCE" >&2
  echo "       run: sh $ROOT/packaging/fetch-binary.sh" >&2
  exit 1
}

have_config() { [ -f "$CONFIG_DIR/config.toml" ]; }

# `kampr serve` exits 1 with no config, and the unit restarts on failure with no limit — so
# starting an uninitialised node is a five-second loop, forever, against an error no supervisor
# can fix. Every path that starts the service goes through one of these two.
init_config() {
  if have_config; then return 0; fi
  "$BIN" init --config-dir "$CONFIG_DIR" --state-dir "$STATE_DIR"
}

require_config() {
  if have_config; then return 0; fi
  echo "kampr: no config at $CONFIG_DIR/config.toml, so the node has nothing to serve." >&2
  echo "       run: sh $ROOT/packaging/kamprctl.sh setup" >&2
  exit 1
}

case "${1:-status}" in
  setup)
    require_binary
    init_config
    install_unit
    if ! svc start; then
      echo "kampr: could not start $UNIT — 'kampr doctor' says why" >&2
    fi
    exec "$BIN" setup --config-dir "$CONFIG_DIR" --state-dir "$STATE_DIR"
    ;;
  # Startup hooks are one-shot, not supervised, so this only ensures the unit exists and is running.
  # A healthy node makes it a no-op.
  nudge)
    stage_binary
    if [ ! -x "$BIN" ] || ! have_config || [ "$SUPERVISOR" = none ]; then exit 0; fi
    install_unit
    svc start >/dev/null 2>&1 || true
    ;;
  start)   require_binary; require_config; install_unit; svc start ;;
  stop)    svc stop || true ;;
  restart) require_binary; require_config; install_unit; svc restart ;;
  status)  require_binary; "$BIN" status --config-dir "$CONFIG_DIR" || true; svc status || true ;;
  url)     require_binary; exec "$BIN" url --config-dir "$CONFIG_DIR" ;;
  update)
    sh "$ROOT/packaging/fetch-binary.sh"
    stage_binary
    require_config
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
