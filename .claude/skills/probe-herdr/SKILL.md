---
name: probe-herdr
description: Look up or establish a fact about herdr's behaviour. Use before relying on anything herdr does (an RPC, an event, a dialog, focus, sizing, labels, agent status), when a comment cites a probe row like #233, or when a fact is missing and must be measured and appended to docs/03-probe-log.md.
---

# Probing herdr

`docs/03-probe-log.md` is the only accepted source of facts about herdr. It is 600 KB, so search it
rather than reading it:

```
scripts/probe 233 331        rows by number, under their section
scripts/probe observe blank  rows containing every word
scripts/probe --sections     the topic index
scripts/probe --cites 233    every file that cites a row
```

## When the fact is not there, measure it

1. Run a throwaway named session (`herdr server --session kampr-probe-<n>`) and tear it down
   afterwards. A node serves every herdr session it can find, so a stray one pollutes the next test.
2. Drive it with the tooling in `research/probe/`: `rpc.py` for the socket API, `ptyclient.py` for
   a pane's bytes, `vt.py` to render a herdr TUI's own output. Keep any new probe script there.
3. Record the herdr version when the behaviour could differ between 0.8.2 and 0.9.x.

## Appending a row

- Columns are `| # | Claim | How | Result |`. *How* must be enough to rerun the probe.
- Take the next number after the highest one in the file. **Never renumber** an existing row,
  because code and docs cite numbers as permanent identifiers.
- To supersede a row, strike it through (`~~…~~`) and point it at the row that replaces it. Don't
  edit what an existing row says.
- If other tasks may be appending in parallel, propose the row unnumbered and let one writer
  assign it.
- Cite the row as `#N` in the comment above the code that depends on it.

`crates/kampr-testkit/tests/guards/probe_log.rs` fails on a gap, a duplicate, a citation past the
end, or a row from `origin/main` that was edited without being struck through.
