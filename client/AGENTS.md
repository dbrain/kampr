# client/

Compose Multiplatform: `shared`, the three pane surfaces (`terminal`, `conversation`, `mosaic`) and
the composition roots (`androidApp` and `webApp` ship, `desktopApp` is for development).
`terminal-spike` is the renderer feasibility spike, kept for `FINDINGS.md`; `tools/fixture-node.py`
is a pre-node wire fixture from the same era. Neither is on a shipping path.

## Running it

- **`env -u GRADLE_HOME ./gradlew`**, always, from here. A `GRADLE_HOME` on this machine silently
  overrides the wrapper (#67). The repository's `make` targets and its Claude Code hook enforce it.
- `make check-client` (from the root) is exactly the CI client job: `check`, the bundle the node
  embeds, a debug APK, and a refusal to accept a test result served from the build cache.
- Kotlin style is ktlint, applied to every module from `build.gradle.kts` with no baseline: the
  tree is clean, so `ktlintCheck` must stay empty. `ktlintFormat` fixes most of what it reports.

## Traps that have cost real time

- **A `FontFamily` of loaded fonts draws everything from its first font.** A second face in it
  supplies no glyph the first is missing, either way round (#416) — so a codepoint the terminal
  face lacks is tofu in the browser and nothing else can answer for it. `tools/terminalmono.py`
  (at the repository root) merges glyphs into the four faces; adding a font beside them fixes
  nothing.
- **Text shaping is the whole cost of a frame** (#58–#62, `terminal-spike/FINDINGS.md`). Run-level
  layouts are cached and pinch-zoom scales a layer until it settles; a refactor that shapes per
  frame is a regression however much cleaner it reads.
