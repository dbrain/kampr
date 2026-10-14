---
name: kampr-review
description: House style and review priorities for this repository. Use whenever reviewing a diff, a PR or the codebase here, before filing any finding. Several deliberate conventions look like defects without it.
---

# Reviewing Kampr

## Do not flag these, they are the house style

- **Absent docstrings and comments.** A comment that restates its code is a defect here. A function
  with no doc comment is finished, not undocumented.
- **`#123` in a comment.** That's a citation into the probe log, not a ticket. The claim above it was
  measured, so run `scripts/probe 123` before disagreeing.
- **Constants that look arbitrary.** Most trace to a measurement (a 40-row grid, a 20-reading proof
  lifetime, a 4-cell margin, a 22 dp strip). Search the probe log before calling one magic.
- **Long prose comments explaining *why*.** They are usually the only record of an expensive
  lesson.
- **Prose commit messages and test names that read as sentences.** The test name says which defect
  it guards against.
- **Allocation-free paths in the renderer and emulator.** Text shaping is the whole cost of a frame
  (#58–#62). An allocation per cell or per frame is a regression, however much cleaner it reads.
  `kampr-term/tests/alloc.rs` and the TUI's `render_alloc` test enforce this.

## Worth flagging

- **A wire change that isn't additive.** Old clients run on real phones.
- **Anything that takes a pane's PTY outside the paths in ADR 0015**, or focuses a pane implicitly.
- **A claim about herdr with no probe row behind it**, in code or in a comment.
- **A test that would still pass with the defect restored.** This is the strongest finding you can
  file here. Name the mutation and what should have failed.
- **Dead code**: anything defined but never called outside its own tests.
- **A `catch` or `unwrap_or` that turns a failure into a plausible-looking success.** The costliest
  bug in this project's history was a node that answered correctly while one of its two paths to
  herdr was dead (#233).
