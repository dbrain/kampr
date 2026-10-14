# kampr-herdr

The socket RPC, the snapshot model, the `observe` and `control` children, binary resolution.

## Traps that have cost real time

- **The layout rect is not the PTY.** Neither its width (#68, #84) nor its height (#205) — it is
  the pane's outer box, and the column it keeps back is the scrollbar's (#230). `viewport_rows` is
  honest for the height. Nothing reports the width (#221); `Herdr::pane_width` reads it by bounding
  `pane.selection.read` (#509, ADR 0014). `ARCHITECTURE.md` §4.2 has the whole history.
- **`Controller` takes the PTY with no way to decline** (#17) and a stopped one holds it for ever
  (#20), so release is timed and ends in a kill. Who may call `claim` is ADR 0015's question, not
  this crate's.
