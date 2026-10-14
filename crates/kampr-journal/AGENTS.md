# kampr-journal

Agent transcripts: one adapter per harness, the turn model, facets, the composer and title reads.
`docs/12-harness-briefs.md` and `docs/14-agent-status.md` are the design.

## Traps that have cost real time

- **herdr labels a pane it has no rules for, and then says `idle` for ever.** The label is the
  foreground process's name with no verification (#75); the *state* comes from per-agent
  manifests, and there is no `omp` manifest — so an omp pane reports `idle` through a whole
  working turn and through an open approval dialog (#485). What omp publishes instead is its
  terminal title, whose separator is the run state (#486), and `title_status` is where that is
  read. A harness nobody has measured a title for leaves herdr's answer alone.
- **`pi` is not `omp`.** They share a record grammar and a session path; the held descriptor, the
  tty breadcrumb, the subagents, the title slot and the run state in the title are all oh-my-pi's
  own (#490). One adapter serves both, and every handle above the working directory answers
  nothing on a `pi` pane rather than answering wrongly.
