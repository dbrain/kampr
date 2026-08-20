# 0008 — Two render modes, and never a hand-rolled glyph atlas

- **Status:** Accepted
- **Date:** 2026-08-20
- **Shipped in:** `3042cb1` (the spike and its negative result), `bed30a7` (the shipping renderer)
- **Evidence:** probes [#56–#62](../03-probe-log.md); raw logs in `client/terminal-spike/results/`
- **Depends on:** [ADR 0003](./0003-the-client-contract-is-a-cell-grid.md)

## Context

[ADR 0003](./0003-the-client-contract-is-a-cell-grid.md) makes the client a renderer with a frame
budget. Compose Multiplatform draws the same code on Android, JVM desktop and wasm, and the question
that had to be answered before the client architecture could be committed was whether a cell grid
can hold 60 fps on the two that matter.

It can, and the spike is worth reading for *what the cost turned out to be* rather than for the
verdict. A 26-scenario benchmark, 300 measured frames after 90 warm-up, against a realistic
workload:

| | wasm (real WebGL2) | Android emulator | desktop |
| --- | --- | --- | --- |
| 74×30, cached run layouts | **0.6 ms** | **1.77 ms** | 0.26 ms |
| 200×50, cached run layouts | 0.8 ms | 3.2 ms | 0.48 ms |
| backgrounds only | 0.0 ms | 0.04 ms | — |
| shape every run, every frame | 8.8 ms | 14.17 ms | — |

Zero dropped frames of 300, everywhere, in the cached configuration.

**Fill is free and text shaping is the entire cost.** That single fact determines everything. It
means caching shaped run layouts is not an optimisation but a requirement: shaping every frame is
~53 % of a 16.67 ms budget on wasm and ~85 % on the Android emulator, and at 200×50 it drops 16–35 %
of frames (#58, #59). The measured cache hit rate in practice is **99.2 %** — terminal output
repeats run strings far more than intuition suggests.

Two consequences followed immediately, and they pull in opposite directions.

**Zoom must not re-shape during a pinch.** Re-shaping at intermediate zoom levels collapses the hit
rate to ~51 %, taking wasm to 11.6 ms with 8.5 % dropped at 200×50 (#60). Layer-scaling during the
gesture is 0.6–3.3 ms with zero drops.

**And the cache has a worst case.** Under cache-hostile churn, cached runs fall to 30 fps with
46–57 % of frames dropped. So the cache cannot be the only path.

The obvious answer to churn is a glyph atlas: rasterise each (glyph, style) pair once into a bitmap
and blit per cell. **It was built, and it is a genuine negative result worth preserving** (#61):

| | Result |
| --- | --- |
| Android | **2.53 ms — the fastest mode measured, of any mode, anywhere** |
| JVM desktop | **192.8 ms/frame — 2.2 fps** |
| wasm | **`RuntimeError: Aborted()` from Skia, every frame** |

Rendering was *visually correct* in all three. This is not a bad idea implemented badly; it is a
correct idea reaching Skia through the wrong door. `DrawScope.drawImage` is cheap on Android and
catastrophic on skiko at ~2 200 calls per frame. **Skia already keeps its own GPU glyph atlas** —
the way to reach it from common code is per-glyph `drawText`, not a second atlas layered on top of
the first.

And per-glyph `drawText` turns out to be exactly the churn escape hatch (#62): it holds 60 fps with
zero dropped frames on the worst case where cached runs collapse.

## Decision

**The renderer has two modes and switches between them on measured cache health. It does not
hand-roll a glyph atlas, on any platform, including the one where that was fastest.**

- **Cached run layouts by default.** Runs coalesce on (foreground, font key); the layout is cached
  and the colour applied at draw time, so the same shaped run serves every pen that uses it.
- **Per-glyph `drawText` when a frame's hit rate collapses**, reaching Skia's own atlas through the
  path common code has.
- **The switch is hysteretic**, and it has to be: drop below a 0.70 smoothed hit rate, rise above
  0.90, over an eight-frame exponential window with a minimum sample count. A single threshold would
  oscillate on exactly the workloads that need the fallback.
- **In per-glyph mode the run cache cannot report**, because nothing populates it — so its hit rate
  can never recover on its own and would strand the renderer in the fallback forever. The recovery
  signal is instead frame-to-frame **overlap of the run-key set**, which is a lower bound on what
  the cache would have achieved. A lower bound is what makes coming back safe.
- **Pinch drives a `graphicsLayer` and folds into committed zoom on settle.** Pan writes straight to
  the origin rather than into the layer, because translating an already-painted surface leaves the
  newly revealed rows blank until the finger lifts.
- **Android must never rasterise its own atlas even though that is its fastest mode.** One renderer,
  three platforms; a platform-specific fast path that aborts on another platform is not a fast path,
  it is a fork.

## Consequences

- **The shipping renderer is faster than the spike it was derived from**, which is not the usual
  direction: draw p50 went 0.6 → 0.40 ms on wasm and 1.77 → 1.37 ms on Android, with zero dropped
  frames. Under the churn case the mode switch is the difference between 46 % dropped frames and
  none.
- **Two thousand rows of history above a live grid costs 0.7 ms**, because only visible rows are
  ever read out of the surface.
- **Android's headroom is not banked.** Every Android number is from an x86_64 emulator on a desktop
  i7 with GPU passthrough, which flatters the CPU-bound shaping cost. Even allowing a further 3×,
  74×30 sits at ~5 ms of a 16.67 ms budget — but **a real mid-range ARM phone is still owed**, and
  it is the shaping cost that will move.
- **Font handling is now load-bearing infrastructure rather than a detail.** Three separate hazards
  came out of the spike, and one of them measured the wrong font for an entire benchmark run: KMP
  library targets silently omit `compose.components.resources` assets from an APK (#64); resource
  fonts resolve asynchronously and can beat the first cell-metrics probe (#65); and ligatures
  collapse two cells into one glyph inside a shaped run, so the no-ligature cut is mandatory (#66).
  The first was caught by screenshotting, not by the numbers — which is the lesson.
- **Font resolution is gated on skiko and re-probed on Android.** The stated rule is to gate first
  paint on the font resolving; the Android implementation returns a family synchronously and relies
  on a re-probe loop to correct the cell metrics instead. That is a divergence between the rule and
  one platform, and it is the kind that produced #64 in the first place.
- **The measured hit rate is off by one glyph per frame**, because the cursor is painted after the
  frame's sample is taken. Cosmetic, recorded so nobody re-derives it from a confusing counter.

## What would justify revisiting

- **A real mid-range ARM phone missing the budget.** The first response is not a new render mode —
  it is to check whether the fallback is engaging, and at what hit rate. The hysteresis thresholds
  are the tuning surface; the mode set is not.
- **Compose Multiplatform gaining a text-run batch API, or skiko fixing `drawImage` throughput.**
  The atlas idea is sound and only the API path defeated it. If either changes, the shape to
  re-measure is the *existing* atlas branch in the spike, not a new one — the spike is still in the
  tree with its results, precisely so that experiment does not have to be re-run from scratch.
- **A WebGPU path in skiko.** `navigator.gpu` exists in Chromium 151 and skiko does not use it
  (#57). If it ever does, every number in this ADR is stale.
- **A second client on a platform Compose does not reach.** Then the mode selector is not the thing
  to port; the cell-grid contract is, and this ADR is scoped to one renderer.
