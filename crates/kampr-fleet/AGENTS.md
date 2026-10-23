# kampr-fleet

One command across the herd: a native session with a cohort, the waiting ladder, the fleet provider.
`docs/13-fleet-runs.md` is the design.

## Traps that have cost real time

- **A pane's state can only be read by its own parent, at its own privilege.**
  `/proc/<pid>/syscall` is EPERM to anything that did not fork the job (#331), and a job running as
  root refuses `wchan` and `fd/0` too (#332) — so the node can see *nothing* about a `sudo` command
  in a herdr pane. This is why a fleet run is a native session, whose keeper forked it and answers
  `state` with those reads, rather than a pane the node asked herdr for. The node never reads the
  pty either: the log is the keeper's, and a borrowed master is for writes and `tcgetattr` only.
- **A run outlives the node.** Tests that start one must kill its keeper on the way out (see
  `tests/it/main.rs`), because dropping the provider deliberately leaves it running.
- **ble.sh leaves an idle pane's tty with ECHO already off** (#333), so the termios bit that looks
  like a definitive password signal means nothing on a shell pane. It is honest on a pty with no
  shell on it (#337) — and on a fleet run's, which does have one: ble.sh declines to load into any
  `-c` invocation. `docs/13-fleet-runs.md` §"The shell, and the three things it was not allowed to
  break".
- **A login shell's `PATH` is not where `kampr` is installed**, on two of the four machines here.
  A fleet run gets the operator's login `PATH` rather than the service manager's (#392) and that
  is still not enough: the profile that adds `~/.local/bin` is `.bashrc`, which `-l` does not read
  (#419). The directory `current_exe()` is in is appended for exactly this, and `kampr doctor`
  refuses to say `ok` about a fleet `PATH` it cannot resolve `kampr` and `herdr` on.
