# Docker containers

**Status: built (2026-11-09).** Adoption is measured end to end against a real daemon
(`crates/kampr-node/tests/it/native/adopt.rs`); arm64 hosts and images are untested. A node lists the Docker containers on its machine beside its panes,
says why one crashed and what it is using, opens a shell in one, and on request *adopts* one: injects
its own binary and runs a Kampr node inside it, linked to the host over the `docker exec` pipe, so the
container is a host of its own in the herd. Nothing is installed in an image and nothing in a
container listens on anything.

The machines Kampr runs on are ones where its user can already run `docker`. Everything here exposes
what that user could see in a terminal, env included, and no op asks for confirmation.

## What Docker gives us

Measured against Engine API 1.56 on the dev box (2026-11-09):

- **Events are a 256-entry ring.** Health checks are `exec_create/exec_start/exec_die` triples every
  couple of seconds per container, so `events?since=24h` reached back **39 s**. History is ours to
  keep: the node follows `/events` and journals what matters.
- **A restart policy restart is `die` then `start`, never `restart`.** `restart` is only the
  explicit `docker restart`. `RestartCount` counts policy restarts since the last manual start.
- **OOM** is an `oom` event before `die exitCode=137`, and `State.OOMKilled: true` on inspect.
- **`State.Health.Log` holds the last 5 probes with their output**, so an unhealthy container says why.
- **`memory_stats.usage` includes page cache**: 9.6 GB for a postgres whose `anon` was 44 MB. Used
  memory is `usage - stats.inactive_file` (cgroup v2; `total_inactive_file` on v1), as `docker stats` does.
- **Logs** come from `GET /containers/{id}/logs` with the 8-byte stream-multiplex header when the
  container has no TTY; `json-file`, `local` and `journald` drivers can be read back.
- **The released `kampr` is static-pie** and runs in Alpine (musl) and Debian images as copied. In a
  container with no systemd and no `$XDG_RUNTIME_DIR`, `kampr new` already works: the keeper takes
  `Launch::Detached`, roots at `/tmp/kampr-<uid>/s`, and is reparented to the container's PID 1. A
  PID 1 that does not reap (no `--init`) leaves zombies; `pid_alive` already reads `Z` as dead.
- Image architecture is `docker image inspect --format {{.Architecture}}` (`amd64`, `arm64`).

## Wire (additive)

`NodeEntry` gains three optional fields, and `PaneEntry` one:

```jsonc
{ "id": "01JB…", "name": "front", …,
  "docker": true,               // ABSENT: no docker source. false: the socket is configured but unreachable
  "containers": [ {             // ABSENT when empty
    "id": "f49c60fac115…",      // full id
    "name": "kobbler-postgres-test-1",
    "image": "postgres:18.6",
    "state": "running",         // created|running|paused|restarting|exited|dead|removing
    "health": "healthy",        // ABSENT without a healthcheck; starting|healthy|unhealthy
    "started_at": 1794219336,   // unix seconds
    "finished_at": 1794212000,  // ABSENT while it never finished
    "exit_code": 137,           // ABSENT while running
    "oom": true,                // ABSENT when false
    "restarts": 3,              // RestartCount, ABSENT when 0
    "crashes": 4,               // non-zero exits journalled in the last 24 h, ABSENT when 0
    "last_crash": { "at": 1794219337, "exit_code": 137, "oom": true },   // ABSENT when none journalled
    "project": "kobbler",       // com.docker.compose.project
    "service": "postgres-test", // com.docker.compose.service
    "ports": ["5432→5432/tcp"],
    "child": "01JB…-c-f49c60fac115",  // ABSENT unless adopted: the node id of the Kampr node inside it
    "adopt_error": "kt-db no longer exists; release it …"   // ABSENT unless the adoption is failing
  } ],
  "parent": "01JB…"             // on a node: the host node that adopted it (stamped by the host, never claimed)
}
```

`PaneEntry.container` (container name) marks a host pane that is a shell into that container.

Everything else rides on manage ops, so it relays through any number of hubs with the `rid` machinery
that is already there:

| op | fields | reply (on the `managed` ack) |
|---|---|---|
| `docker.detail` | `node`, `id` | `detail`: the entry above plus `command`, `created`, `restart_policy`, `limits {memory, nano_cpus}`, `env[]`, `labels{}`, `mounts[{src,dst,mode}]`, `networks[]`, `health_log[{start,end,exit_code,output}]`, `logs[{at,stream,line}]` (last 200), `stats {cpu_pct, mem_used, mem_limit, net_rx, net_tx, blk_read, blk_write, pids}` (running only), `events[{at, action, exit_code?, signal?, health?}]` (journal, last 100) |
| `docker.shell` | `node`, `id` | `id`: a native pane into the container, labelled with its name; `shell`: `bash`, `sh` or `debug` (see **Shell**) |
| `docker.start` / `docker.stop` / `docker.restart` | `node`, `id` | — |
| `docker.adopt` / `docker.release` | `node`, `id` | adopt: `node` = the child's node id once its herd is listed |

`docker.detail` is read-only and polled every 3 s while a detail sheet is open, so it is not written
to the audit log. The hello caps gain `docker: true`.

## Node

- **`crates/kampr-docker`**: an Engine API client over `/var/run/docker.sock` (`[docker] socket`,
  else `$DOCKER_HOST` when it is `unix://`) on `hyper` + `hyper-util` (already in the lock through
  reqwest). A `Directory` follows `/events?filters={"type":["container"]}` with backoff from 250 ms to
  5 s, re-lists on every relevant event (ignoring `exec_*`), and bumps a revision only on a real change,
  the way `kampr-muffit`'s directory does. A socket that is down is a supported state: no containers,
  `docker: false`, never an error in the herd.
- **The journal** is `<state>/docker/events.jsonl`: `start`, `die`, `oom`, `kill`, `restart`, `stop`,
  `health_status`, `destroy`, each `{at, id, name, action, exit_code?, signal?, health?}`, trimmed to
  the last 14 days or 2 MB. On start the follower asks for `since=<last journalled>` so the ring's
  39 s covers a node restart.
- **`[docker]`** in config: `enabled`, `socket`, `binary`, `fetch`, `debug_image`. Unset `enabled` means on when the socket
  file exists, and `docker` appears on the node once the daemon has answered; `enabled = true` shows
  `docker: false` while it does not. Only the primary session carries it, as with muffit.
- **Shell.** `docker.shell` runs `docker` with `DOCKER_HOST=unix://<resolved socket>`, so the CLI
  reaches the daemon the node follows. A die within 2 s of a `kill` or `stop` is not a crash: that is
  what `docker stop` and `docker kill` emit. Journal lines carry `nano` too, because a crash loop dies
  several times within one second. What it runs is decided before anything is spawned:
  1. A container that is not running is refused: `<name> isn't running — start it first`
     (`<name> is paused — unpause it first`).
  2. The first of `/bin/bash`, `/bin/sh`, `/busybox/sh` (distroless `:debug`) the container has,
     asked of the daemon with `HEAD /containers/{id}/archive?path=<p>` (404 is missing), runs as
     `docker exec -it -e TERM=xterm-256color -w <WorkingDir> <id> <shell> -l`. **The stat is of the
     link, not its target** (measured: a `/bin/bash -> /nowhere` answers 200 with `linkTarget`), and
     the daemon resolves `linkTarget` to an absolute path (`/bin/sh -> dash` reads `/usr/bin/dash`),
     so links are followed, eight hops at most, and the end must be an executable file. `shell` on
     the ack is `bash` or `sh`.
  3. **No shell at all** (scratch, distroless) gets a debug sidecar, the `docker debug` pattern:
     `docker run -it --rm --name kampr-debug-<12 hex>-<6 hex> --pull never --pid container:<id>
     --network container:<id> --cap-add SYS_PTRACE <debug image> sh -c 'cd /proc/1/root; exec sh -l'`.
     It sees the target's processes and sockets, and its filesystem at `/proc/1/root`, where it
     starts. The pane is labelled `<name> · debug`, still carries `container`, and `shell` is
     `debug`. The image is `[docker] debug_image`, `busybox:1.37-musl` by default (it must have
     `sh`), pulled with `POST /images/create` only when the daemon lacks it; a pull that fails
     refuses the op: `<name> has no shell, and the debug image <image> could not be pulled: <why>`.
  Measured (Engine 29.9.3, runc, 2026-11-09):
  - `-w /proc/1/root` is refused by runc ("current working directory is outside of container mount
    namespace root -- possible container breakout detected"), hence the `cd`.
  - Docker's default capabilities read `/proc/1/root` of a target running as root, but not of one
    running as another uid (`-u 1000`: permission denied); `SYS_PTRACE` is what makes it readable.
    `--cap-drop ALL` fails even for a root target.
  - **The sidecar ends with its target**: it lives in the target's pid namespace, so the target's
    pid 1 dying kills it and `--rm` removes it (gone within a second of `docker kill`).
  - **It ends with its pane**: closing a native pane hangs up its process group, and a SIGHUP'd
    `docker run -it` takes its container down. A `docker run` CLI killed with SIGKILL leaves the
    sidecar running until its target stops.
- `herd.rs::same_node` compares the new fields, the wire-shapes fixture is blessed, `docs/04` gets the
  fields and ops.

## Adoption

A host adopts a container by running a Kampr node inside it whose stdin and stdout are a mesh link
to the host.

1. **Inject.** `PUT /containers/{id}/archive?path=/&copyUIDGID=1` with a tar of `.kampr/` (the
   directory entry makes it), `.kampr/kampr` and `.kampr/version`, so the node needs no `docker` CLI
   for it. The binary is the first of these that would run in the image, not naming an ELF
   interpreter (`PT_INTERP`: not static) and built for its `Architecture` (`amd64`/`x86_64`,
   `arm64`/`aarch64`): `[docker] binary` when it is set, the node's own executable, the cached
   release, and a download of it. The release is **this node's own version**, the published
   `kampr-<arch>-unknown-linux-musl` asset, fetched and verified by the installer embedded in the
   binary exactly as `kampr update` verifies one (`crates/kampr-node/src/installer.rs`,
   `KAMPR_MODE=fetch`, `KAMPR_TARGET`), into `<state>/docker/bin/<version>/<target>/kampr` once and
   reused for every container after. That is what lets a Mac running Docker Desktop, an x86 host
   adopting an emulated arm64 container, or a glibc dev or `cargo install` build adopt at all. It
   is fetched on the host, not in the container, because images often have no curl, no CA
   certificates or no network. `[docker] fetch = false` stops at the cache. A version with no
   published release (a dev build ahead of its tag) is refused naming the version and target, and
   says to set `[docker] binary`. The download happens inside `docker.adopt`, which is served off
   the dispatch loop, one fetch at a time so two adoptions download once.
   Refused with a message, before anything is copied or remembered, when nothing would run, when
   the container is not running, and when its root filesystem is read-only
   (`HostConfig.ReadonlyRootfs`, compose's `read_only: true`): `<name> has a read-only root
   filesystem (read_only: true), so kampr can't be copied into it — adopting needs a writable /.
   Shell in still works.` A copy the daemon refuses says so in words naming the container (no space
   left, permission denied, stopped meanwhile) rather than as an API error. `/.kampr/version` holds the binary's size and mtime in nanoseconds; it is read back
   (`GET …/archive`) before every spawn and the binary copied again when it differs, which covers a
   host upgrade and a recreated container alike.
2. **Spawn.** `docker exec -i -e KAMPR_PEER_STDIO <name> /.kampr/kampr serve --peer-stdio
   --config-dir /.kampr/config --state-dir /.kampr/state`, with `DOCKER_HOST` set to the socket the
   node follows. The bare `-e NAME` copies the variable from the host's environment, so the child's
   secret is never in an argv. It addresses the container by **name**, so a recreated container is
   the one reached. The env carries the child's identity, minted and kept by the host: its node id,
   `<host id>-c-<12 hex of the container id at first adoption>`, is kept beside the name in
   `adopted.json` and its key in `<state>/docker/keys/<name>.key`, so both survive respawns and
   recreation. Then its name (the container's) and the host's public key to pin. `docker.adopt` acks
   once the herd lists the child, or after 60 s.
3. **The child** writes its config and state under `/.kampr/`, binds no HTTP, logs to stderr, and serves
   the host over stdio with the same `serve_hub` code a dialled hub gets (made generic over
   `Outgoing`/`Incoming`). It pins the host's key, so `mesh.challenge` from anything else is refused.
4. **The host** enrols the child's key as a peer before spawning, runs the hub side of the handshake
   over the pipes, and serves the link through `Peers::serve` with the link marked *adopted*:
   `publish` stamps `parent` with the host's own id on every node that link advertises.
5. **Lifetime.** Adoptions are remembered by container **name** in `<state>/docker/adopted.json`, so a
   recreated container is re-adopted. The host respawns the child with backoff when the exec ends and
   the container is running, and on every change the directory reports while it is down (a `start`
   among them), so a restarted container is back in seconds rather than at the end of the 30 s idle
   wait. A node starting supervises every remembered adoption again. `docker.release` stops the
   child, forgets the adoption, revokes and deletes its key, and drops the node from the herd rather
   than leaving it listed offline; native sessions inside keep running until the container stops.
   **An adoption that cannot come back says so.** The container's entry carries `adopt_error`, a
   sentence, while the supervisor's last attempt failed: the copy was refused (the check before
   every spawn, on a node starting too), the child ended in an error three spawns running, or the
   container has been gone for 20 s (long enough for `docker compose up` to recreate it). It clears
   when the child is up, and while the container is merely stopped. A remembered container the
   daemon no longer has is listed anyway, `state: "missing"` with `id` its name, so it can be
   released; only while the daemon answers, since a daemon that is down lists nothing. The hub
   sends an `adoption` push ([docs/16](./16-notifications.md)) when one starts failing, once per
   episode, and takes it down when it recovers.
6. **Keepers in containers.** `SessionDir::alive` trusts a pid; a restarted container reuses low pids,
   so `KeeperInfo` records the keeper's `/proc/<pid>/stat` start time and `alive` compares it. Detached
   launches drop `NOTIFY_SOCKET`/`LISTEN_*` from the keeper's environment.
7. **Zombies.** A keeper launched detached is reparented to the container's PID 1. One that does not
   reap (a container run without `--init`, or `init: true` in compose) keeps every ended keeper and
   shell as a zombie until the container restarts. `pid_alive` reads `Z` as dead, so nothing
   misreports; `kampr doctor`'s docker line names the adopted containers without `--init`.

A static build for a test: `kampr_testkit::static_kampr()` builds `kampr-cli` for the host's
`*-unknown-linux-musl` target (dev profile, `vendored-tls`) with `clang --target` against a sysroot of
Alpine's `musl-dev` headers, fetched once into `~/.cache/kampr/musl-sysroot-<arch>` (or
`$KAMPR_MUSL_SYSROOT`; `$KAMPR_STATIC_BIN` skips the build). About 2.5 minutes cold, 145 MB. A
glibc build does not run in these images: Alpine has no glibc and Debian's is older than the host's.

### Through a hub

A host that is itself a peer passes its children up already (a hub forwards whatever its links
advertise). What a chain needs and nothing tests yet: a `~cols` reflowed view relayed a second hop
(the key may double), and `att.*`/`up.*` for a pane that is not the middle node's own are served from
its disk instead of forwarded. Both get a two-hop test and a fix.

## Client

- Per host, under its terminals (which stay plain rows): **Background · muffit** and **Containers · docker**,
  both collapsible, the state remembered per device in prefs keyed by `(node id, section)`. Muffit
  starts expanded, containers collapsed. One header composable serves both.
- Containers group under their compose project when they have one (a caption, not a level). Shown
  by default: running, restarting, paused, unhealthy, or crashed within 24 h; the rest sit behind one
  **+N stopped** row that toggles them (also remembered).
- A row: state dot, name, then uptime, or `exited 137 · OOM 3m ago`; badges `↻4` (restarts) and
  `unhealthy`. Shells into it (`PaneEntry.container`) list under it.
- Tapping a container opens its sheet: state, crash timeline, health output, live CPU and memory
  against the limit, last log lines, env, mounts and ports; **Shell in**, **Restart**, **Stop**/**Start**,
  **Adopt** / **Release**.
- An adopted container's node (`parent` set) is drawn under its container row instead of as a top-level
  host, with its own terminals and sections. Nesting indents by 8 dp with a hairline guide, never more,
  so titles keep their width. An old client shows it as an ordinary top-level host.

### Terminal client

`kampr` (the TUI) draws the same sections in its sidebar, with the same filter, captions and nesting;
folds are saved in `tui.toml`. In the sidebar navigator (`prefix w`): `enter` on a section header or
**+N stopped** folds it (a left click too), `enter` on a container opens its detail panel (refreshed
every 3 s), and `s` shell in, `r` restart, `x` stop/start, `a` adopt/release — also inside the panel,
with `j`/`k`, PgUp/PgDn and `esc`/`q`. The herd triage screen still lists panes flat.

## What older builds do

An older client ignores `docker`, `containers`, `parent`, `container` and `adopt_error`, shows a
`missing` container by its raw state, and is never sent an `adoption` push (it needs payload v4), and shows an adopted
container as a host of its own and a shell as an ordinary terminal. An older hub decodes `NodeEntry`
into its own struct and drops the new fields on relayed nodes, so containers appear only once the hub
is upgraded; manage ops relay through it unchanged.
