# Terminal renderer spike — can Compose Multiplatform draw a 74×30 cell grid at 60 fps?

**Verdict: viable, with two caveats.** wasm and Android both hold a locked 60 fps at 74×30 with a
realistic terminal workload, with roughly 25× draw-time headroom on wasm and 9× on Android. 200×50
also holds 60 fps. The caveats are (1) run-level text layouts **must** be cached — shaping is ~93 %
of frame cost, and shaping every frame reaches ~53 % of budget on wasm and ~85 % on the Android
emulator; and (2) pinch-zoom must scale a layer during the gesture and re-shape only on settle,
because re-shaping at every intermediate zoom step drops frames at 200×50.

Date: 2026-08-20. Kotlin 2.4.10, Compose Multiplatform 1.11.1, AGP 9.3.1, Gradle 9.7.0, JDK 26
daemon with a JVM 21 toolchain.

---

## Headline numbers

Realistic workload (`mixed`: ~40 rows/s of `grid.patch`, a full `grid.reset` every 2 s, blinking
block cursor), cached run layouts, 300 measured frames after 90 warm-up frames. 60 fps budget =
16.67 ms.

**74 × 30**

| target | frame p50 | p95 | p99 | max | dropped | draw p50 | draw p99 |
|---|---|---|---|---|---|---|---|
| wasmJs, Chromium 151, WebGL2 | 16.70 | 16.79 | 16.79 | 16.79 | 0 / 300 | **0.6** | 1.1 |
| Android **emulator** API 35 | 16.66 | 16.66 | 16.66 | 16.66 | 0 / 300 | **1.77** | 3.55 |
| JVM desktop (skiko) | 16.67 | 17.59 | 17.78 | 18.71 | 0 / 300 | **0.26** | 0.85 |

**200 × 50**

| target | frame p50 | p95 | p99 | dropped | draw p50 | draw p99 |
|---|---|---|---|---|---|---|
| wasmJs | 16.70 | 16.79 | 16.79 | 0 / 300 | **0.8** | 1.4 |
| Android emulator | 16.66 | 16.66 | 16.66 | 0 / 300 | **3.2** | 4.23 |
| JVM desktop | 16.67 | 17.01 | 17.74 | 0 / 300 | **0.48** | 1.66 |

`scroll` (30 lines/s, every row rewritten on each new line — the real cost of terminal scrolling)
is cheaper than `mixed`, because a scrolled screen reuses run strings: wasm 0.7 ms, Android 3.0 ms,
desktop 0.37 ms at 200×50, zero dropped frames on all three.

Frame interval is pinned to vsync on every target.

---

## Where the time actually goes

Measured by differential, not guessed. `draw p50` at 74×30, `mixed` workload, milliseconds.

| what is being drawn | wasm | Android emu | desktop |
|---|---|---|---|
| backgrounds only, no text (`no-text`) | 0.0 | 0.04 | 0.01 |
| text, shaping every run every frame | 8.8 | 14.17 | 3.51 |
| text, cached run layouts | 0.6 | 1.77 | 0.26 |
| text, cached per-glyph layouts (one `drawText` per cell) | 2.6 | 7.58 | 1.43 |
| cached runs, whole grid composable recomposed every frame | 0.4 | 1.51 | 0.17 |
| model update (message build + apply into the cell buffer) | 0.0 | 0.05 | 0.01 |

- **Fill is free.** 2 220 background rects plus a full-grid clear costs under 0.05 ms everywhere.
  The GPU is nowhere near being the constraint.
- **Text shaping is the entire cost.** 8.8 → 0.6 ms on wasm is a 93 % reduction from caching run
  layouts alone; 14.17 → 1.77 ms on Android is 88 %. Cache hit rate on the realistic workload is
  **99.2 %** — terminal output repeats its run strings far more than intuition suggests.
- **Recomposition is not a factor.** Deliberately reading a frame counter in composition so the
  grid host recomposes every frame is *within noise* of draw-only invalidation (0.4 vs 0.6 ms wasm;
  1.51 vs 1.77 ms Android). The grid is one `Spacer` with a `Modifier.drawBehind`, so there is no
  subtree to re-run. Engineering around recomposition is not a lever worth pulling here.
- **Buffer diffing is not a factor.** Applying `grid.reset` / `grid.patch` into `CharArray` +
  `ShortArray` is 0.00–0.05 ms at 74×30, and 0.4–2.7 ms only in the pathological 200×50 case where
  10 000 cells are rewritten every frame. The wire model in `04-wire-protocol.md` (style table +
  run-length rows) expands into a flat cell buffer essentially for free.
- **Per-glyph layouts are ~4× worse than per-run layouts** on the realistic workload — you trade
  ~300 `drawText` calls for ~2 200. Keep it as a fallback, not a default.

---

## Graphics backend — confirmed, not assumed

Probed at runtime from inside the page rather than inferred:

```
canvases=1 | bodyChildren=div |
canvas0=webgl2 renderer=ANGLE (Intel, Mesa Intel(R) UHD Graphics (TGL GT1), OpenGL ES 3.2)
css=1400x860 | webgpu_api=present | dpr=1
```

**WebGL2 through ANGLE on the integrated Intel GPU.** Not a Canvas 2D fallback, not SwiftShader.
Every wasm number here was measured on that backend. `graphicsBackend()` in `Platform.wasmJs.kt`
walks the DOM (including shadow roots) and reports this as a `KAMPR_ENV` line at the top of every
run, so any future run states which backend it got.

`navigator.gpu` is present in Chromium 151 but Skiko does not use it. Compose Multiplatform 1.11.0's
release notes mention only "Update Skia to m144" and say nothing about WebGPU or a new web backend
(<https://github.com/JetBrains/compose-multiplatform/releases/tag/v1.11.0>); CMP's web target is
documented as Skia-over-WebGL with a Canvas 2D fallback. There is no WebGPU switch to flip in
1.11.1, and since fill is already free it would buy nothing here.

Android's backend is HWUI (platform-chosen Vulkan or GLES); the emulator ran with `-gpu host`.

---

## Pinch-zoom and pan

Kampr never resizes a pane, so zoom is the whole small-screen story. Two strategies, measured under
a scripted gesture (zoom oscillating 0.75×–1.85× plus pan, changing every frame — harsher than a
human pinch).

| strategy | wasm 74×30 | wasm 200×50 | Android 74×30 | Android 200×50 |
|---|---|---|---|---|
| re-shape at each new font size (sharp) | 7.5 ms, 0 dropped | 11.6 ms, **8.5 % dropped** | 9.38 ms, 0 dropped | 14.09 ms, **2.9 % dropped** |
| `graphicsLayer` scale of the base render | 0.6 ms, 0 dropped | 0.8 ms, 0 dropped | 1.92 ms, 0 dropped | 3.26 ms, 0.3 % dropped |

Re-shaping is sharp but thrashes the layout cache — hit rate collapses from 99 % to ~51 %, and every
frame re-shapes every run at a new size. Layer scaling costs nothing but resamples the text, so it
is slightly soft while the fingers are down.

**Recommendation: layer-scale during the gesture, re-shape once when it settles.** At 74×30 you
could get away with re-shaping live; at 200×50 you cannot. Pan alone is free either way — it is an
origin offset, no re-shaping.

---

## Worst case

`worst` regenerates every cell of every row with random text and a random style **every frame** —
deliberately cache-hostile, and far beyond anything a real pane produces (74×30 at 60 fps is
133 000 styled cells per second).

| 74×30 worst | wasm | Android emu | desktop |
|---|---|---|---|
| shape every frame | 33.3 ms p50, 42 % dropped | 33.33 ms p50, 57 % dropped | 16.72 ms p50, 9.6 % dropped |
| cached run layouts (0.25 % hit) | 33.3 ms p50, 46 % dropped | 33.33 ms p50, 57 % dropped | 16.71 ms p50, 12 % dropped |
| **cached per-glyph layouts** | **16.70 ms p50, 0.3 % dropped** | **16.66 ms p50, 0 dropped** | 16.68 ms p50, 0 dropped |

When run caching cannot hit, per-glyph layouts win decisively: the glyph set is small and bounded,
the run set is not. **Ship both** — cache runs by default, fall back to the per-glyph path when the
observed run-cache hit rate for a frame drops below a threshold. That is ~30 lines of switch in
`GridRenderer` and it converts the pathological case from 30 fps to 60 fps.

200×50 worst holds on no target (wasm 7.5 fps, Android 6 fps, desktop 16.5 fps). That is 600 000
styled cells per second; it is not a scenario Kampr has to serve.

---

## Glyph atlas — tested, and a negative result on Skia

Implemented properly (`GlyphAtlas.kt`): each unique (glyph, bold/italic) is rasterised once, white,
into a shared `ImageBitmap`; cells are blitted with `drawImage` +
`ColorFilter.tint(fg, BlendMode.SrcIn)`, snapped to integer pixel positions, underline/strike drawn
as rects. Exactly the xterm.js-WebGL / alacritty shape.

| 74×30 glyph atlas | draw p50 | result |
|---|---|---|
| Android emulator, mixed | 2.53 ms | 60 fps, 0 dropped |
| Android emulator, **worst** | **3.76 ms** | **60 fps, 0 dropped** — best worst-case result of any mode |
| JVM desktop, mixed | 192.8 ms | 2.2 fps, 96 % dropped |
| wasmJs, mixed | — | `RuntimeError: Aborted()` thrown from Skia every frame |

The atlas concept is right — on Android it is the fastest renderer measured and the only mode that
keeps the pathological workload locked at 60 fps. The problem is the **API path on Skia**. On
Android `DrawScope.drawImage` lands on `android.graphics.Canvas.drawBitmap`, which is cheap. On
skiko (desktop *and* wasm — so the failure is Skia-wide, not wasm-specific) ~2 200 `drawImage` calls
per frame against one `ImageBitmap` are catastrophic; the wasm build aborts outright, consistent
with an allocation failure from re-snapshotting an `Image` from the bitmap on every call. Rendering
was visually correct in both cases, so this is purely a cost/robustness problem, not a logic one.

**Do not hand-roll a glyph atlas through Compose's common `drawImage`.** If one is ever needed on
the Skia targets it must go through skiko-native `Canvas.drawImageRect` with a single cached
`org.jetbrains.skia.Image`, or better `drawTextBlob` with explicit per-cell glyph positions, behind
an `expect`/`actual`.

None of that is needed for the shipping renderer. Skia already maintains its own internal GPU glyph
atlas, and the `glyph-cache` mode is how you reach it from common code: shape each glyph once, one
`drawText` per cell, let Skia blit from its atlas. That is what the worst-case row above measures,
and it is enough.

`bitmap-dirty` (repaint only dirty rows into a persistent `ImageBitmap`, blit once per frame) was
also measured as the other obvious mitigation. It matches or slightly beats cached runs on the
realistic workload (wasm 0.6 ms, Android 0.28 ms at 74×30) but is worse under churn and adds a
full-grid texture upload per frame. Not worth the complexity when cached runs already use 3.6 % of
budget.

---

## Method

- **Model.** `Wire.kt` mirrors `docs/04-wire-protocol.md`: `ColorSpec` (`d` / `i` / `r`), `Style`
  with the boolean attribute set, `Run{s,x,l}`, `RowDiff{row,runs}`, `styles` / `grid.reset` /
  `grid.patch`. `StyleTable` resolves style ids once into `fg`/`bg` ARGB arrays plus a font key
  (bold/italic/underline/strike bits), applying dim, reverse and hidden at resolve time.
  `CellBuffer` is `CharArray` + `ShortArray` + a per-row dirty flag. No JSON — transport cost is out
  of scope for a render spike, so the workload builds message objects in memory.
- **Workload.** `Workload.kt` synthesises realistic pane content: timestamped log lines, shell
  prompts, syntax-coloured code, `+`/`-` diff rows with truecolour backgrounds, block-character
  progress bars, box drawing, and underlined links, across a 40-entry style table with indexed and
  truecolour pens. Profiles: `idle`, `mixed`, `scroll`, `worst`.
- **Renderer.** `GridRenderer.kt` draws through one `Modifier.drawBehind`. Backgrounds coalesce into
  runs of equal bg; text coalesces into runs of equal (fg, font key) with trailing blanks trimmed.
  Cell advance comes from measuring a 32-`M` probe, so the grid is exact. Font is **JetBrains Mono
  NL** — the no-ligature cut; the ligature build breaks cell alignment inside a shaped run.
- **Timing.** A `withFrameNanos` loop records the frame interval; the model update is timed around
  `Workload.step` + `CellBuffer.apply`; the draw is timed around the `drawBehind` body. `FrameStats`
  keeps ring buffers and reports p05/p50/p95/p99/max, a bucketed histogram, and dropped frames
  counted as vsync slots skipped (`round(interval / 16.67) - 1`). The HUD shows the live histogram;
  every scenario also emits a `KAMPR_BENCH` line to console / logcat / stdout.
- **Harness.** `BenchPlan` runs 26 scenarios unattended, 90 warm-up + 300 measured frames each.
  `tools/serve.py` serves the wasm bundle and captures POSTed result lines; `tools/bench-wasm.mjs`
  drives a real headed Chromium through Playwright and scrapes the console.

### What the numbers do and do not include

`draw p50` is the cost of *recording* the frame — text shaping plus canvas command recording. GPU
rasterisation happens after the lambda returns and is not in that figure; it shows up in the frame
interval and dropped-frame counts, which is why both are reported. On wasm `performance.now()` is
coarsened to 100 µs without cross-origin isolation, so wasm draw times have 0.1 ms granularity and
sub-0.1 ms values read as `0.0`. Frame intervals on wasm come from `requestAnimationFrame`
timestamps, which sit exactly on the vsync grid — hence the 16.6 / 16.7 / 33.3 quantisation, and
hence dropped frames are exactly countable there.

### Hardware

| | |
|---|---|
| Host | Intel i7-11800H (8C/16T), 62 GB, Mesa Intel UHD Graphics TGL GT1, Linux 7.1.8 (CachyOS), Wayland, 2560×1440 @ 59.91 Hz |
| Browser | system Chromium 151.0.7922.137, headed, real vsync, 1400×860 viewport, dpr 1 |
| **Android** | **emulator only — `Medium_Phone_API_35`, Android 15, x86_64, `-gpu host`, 1080×2400 @ 60 Hz, 2 GB RAM, 8 vCPU** |
| Desktop | JVM 26.0.2, skiko 0.144.6, 1280×800 window |

**The Android numbers are emulator numbers and must not be read as phone numbers.** No physical
device was attached. The emulator runs x86_64 on a desktop-class CPU with GPU passthrough, so its
CPU-bound costs — which is exactly the text shaping that dominates here — are optimistic for a
mid-range ARM phone. The emulator is already 5× slower than the desktop JVM on shape-every-frame
(14.17 vs 3.51 ms), so it is not simply desktop-speed, but 74×30 should be re-measured on real
hardware before it is treated as settled. Even assuming a further 3× penalty against the emulator,
cached runs at 74×30 would be ~5 ms of a 16.67 ms budget, which still clears.

Run-to-run variance on this host is real: an earlier wasm pass taken while the Android emulator was
still running showed 30–55 % dropped frames on scenarios whose draw time was 0.1 ms — entirely
external interference. The published run was taken with the emulator and all Gradle daemons stopped.
Anything reporting drops alongside a sub-millisecond draw time is contention, not the renderer.

---

## Reproducing

```bash
cd client

# desktop, prints KAMPR_BENCH lines to stdout
./gradlew :terminal-spike:run

# wasm
./gradlew :terminal-spike:wasmJsBrowserDistribution
python3 terminal-spike/tools/serve.py \
    terminal-spike/build/dist/wasmJs/productionExecutable 8731 results.log &
BROWSER_PATH=/usr/bin/chromium node terminal-spike/tools/bench-wasm.mjs \
    http://127.0.0.1:8731/index.html chromium            # needs `npm i playwright`

# android
./gradlew :terminal-spike:androidApp:installDebug
adb shell am start -n dev.kampr.terminal.spike/dev.kampr.terminal.spike.app.MainActivity
adb logcat -s KamprSpike:I
```

Raw output from the runs quoted above is in `results/`.

---

## Things the spike turned up that briefs C and D both need

1. **AGP 9 forbids `com.android.application` together with the Kotlin Multiplatform plugin.** The
   shared module must use `com.android.kotlin.multiplatform.library` and the APK must come from a
   separate, plain-Android module. `client/` is laid out that way (`terminal-spike` +
   `terminal-spike/androidApp`); the real `client/shared` will need the same shape.
2. **`compose.components.resources` does not emit Android assets for a
   `com.android.kotlin.multiplatform.library` target in CMP 1.11.1.** The fonts were prepared for
   `jvm` and `wasmJs` and silently omitted from the APK, so Android fell back to the system sans and
   rendered the grid with proportional glyphs. `terminal-spike/androidApp/build.gradle.kts` carries
   a `Copy` task that stages `composeResources/<pkg>/…` into the APK's assets as a workaround. This
   will bite the design-token and icon work too.
3. **Compose resource fonts load asynchronously, and the first cell-metrics probe can beat them.**
   Measuring the cell advance against the fallback font yields a column pitch unrelated to the
   glyphs eventually drawn — and nothing ever recomputes it. The spike revalidates the probe for the
   first 60 frames of each scenario; the real client should gate first paint on the font being
   resolved. Any renderer deriving geometry from font metrics needs an explicit answer here.
4. **Use the no-ligature font cut.** JetBrains Mono's ligatures (`->`, `!=`, `==`) collapse two cells
   into one glyph and desynchronise the grid inside a shaped run. `JetBrainsMonoNL` is bundled for
   that reason.

## What the shipping renderer should do

- One `Modifier.drawBehind` over the whole grid, invalidated by a frame counter. Do not lay out
  per-character text nodes, and do not bother engineering around recomposition — it is already free.
- Cache `TextLayoutResult` per (run text, font key), draw with the colour overridden at draw time so
  one layout serves every colour. Expect ~99 % hit rate; cap the cache and clear on font-size change.
- Keep a per-glyph layout cache as the fallback path and switch to it when the run-cache hit rate
  for a frame collapses. That is what makes the cache-hostile case survivable.
- Zoom via `graphicsLayer` while the gesture is live; re-shape at the new size once it settles.
- Take the cell advance from the font's own metrics rather than snapping it to integers — fractional
  advance is what keeps a long shaped run aligned to the grid.
