# kampr-node

The HTTP/websocket server, the herd model, sessions and manage ops.

## Testing

`tests/it/live.rs` and `tests/it/mesh.rs` drive a **real** herdr and are the honest level for most
fixes here. They are left out of `make test-fast` and run by `make test`.

- **Live tests need a real herdr on PATH** and are sensitive to machine load and to stray
  sessions — a node serves *every* herdr session it can find (#97). Use a throwaway named session
  and tear it down. Re-run a failure alone before calling it a regression.
- A suite with no herdr panics rather than skipping, and says whether herdr is absent or present
  and not listening (`kampr_testkit`, #233).

## Traps that have cost real time

- **A node reaches herdr two ways** — a socket, and a spawned `herdr terminal session observe`.
  They fail independently, and every surface a person can see is served by the socket, so a node
  whose binary half is broken looks completely healthy with every pane blank (#233). `kampr
  doctor`'s `observe` check answers this directly.
- **Only three paths take a pane's PTY**: `manage.rs` (the `pane.size` op), `restore.rs` (putting a
  matched pane back) and `kampr resize`. ADR 0015 says why; a guard test fails on a fourth.
- **A digit does not answer every dialog.** On Claude's single-answer question it answers and the
  dialog closes (#413); on a multiple-answer one the same digit *ticks a box*, `\r` toggles the
  focused row, and only right-arrow then Enter commits (#421). `pending.multi` is what tells the
  two apart. **omp draws no digits at all** (#487): its dialogs are a `❯` on one row, moved with
  the arrows and committed with Enter, so the node turns the key a client pressed into that many
  moves, counted from a screen read taken at the moment of the press. **Claude's trust prompt lost
  its digits in 2.1.282** (#550), so Claude is read for numbers and then for a cursor.
  `pending::cursor_dialogs` is the per-harness list; a harness not on it is read for numbers only.
