#!/usr/bin/env bash
# N busy loops that expire on their own, torn down early by `spin.sh stop`.
#
# The TTL is the point. Sixteen of these once outlived the experiment that wanted them and held a
# sixteen-core box at load 36 for fifty minutes, through somebody else's working day, because load
# with no deadline is indistinguishable from load somebody meant. An abandoned run now dies by
# itself, and the worst case is bounded by SPIN_TTL rather than by whoever notices.
#
# Teardown goes through a pidfile because neither pgrep mode can see these. `exec -a` sets argv[0]
# but not comm, so a name match looks for "bash"; and `kampr-probe-spinner` is 20 characters, past
# the 15 the kernel keeps, so `pkill -x` refuses it outright. `pkill -f` does match, but it also
# matches this script and the shell running it, so `stop` used to kill its own shell partway down
# the list and print "spinners down" over a machine still carrying spinners.
set -u
pids="${TMPDIR:-/tmp}/kampr-probe-spinner.pids"
alive() { ps -eo args | awk '$1=="kampr-probe-spinner"' | wc -l; }
case "${1:-start}" in
  start)
    ttl="${SPIN_TTL:-900}"
    : > "$pids"
    for _ in $(seq "${2:-16}"); do
      (SPIN_TTL="$ttl" exec -a kampr-probe-spinner \
        bash -c 'end=$((SECONDS+SPIN_TTL)); while [ $SECONDS -lt $end ]; do :; done') &
      echo $! >> "$pids"
    done
    echo "spinners up: $(wc -l < "$pids"), expiring in ${ttl}s" ;;
  stop)
    [ -f "$pids" ] && xargs -r kill -9 < "$pids" 2>/dev/null
    rm -f "$pids"
    left=$(alive)
    echo "spinners left: $left"
    [ "$left" -eq 0 ] || exit 1 ;;
  status) echo "spinners alive: $(alive)" ;;
esac
