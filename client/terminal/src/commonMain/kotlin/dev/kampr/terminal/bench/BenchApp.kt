package dev.kampr.terminal.bench

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.BasicText
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.theme.KamprTheme
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.terminalPalette
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.terminal.render.GridRenderer
import dev.kampr.terminal.render.ModeSelector
import dev.kampr.terminal.render.RenderMode
import dev.kampr.terminal.render.ResolvedStyles
import dev.kampr.terminal.render.SurfaceRows
import dev.kampr.terminal.render.TextCache
import dev.kampr.terminal.view.BASE_CELL_SP
import kotlin.math.cos
import kotlin.time.TimeSource

private const val BUDGET_MS = 1000f / 60f
private const val BENCH_PANE = "bench/w1:p1"

enum class ZoomMode { None, Redraw, Layer }

class Scenario(
    val name: String,
    val cols: Int,
    val rows: Int,
    val profile: Profile,
    val forced: RenderMode? = null,
    val zoom: ZoomMode = ZoomMode.None,
    val history: Int = 0,
)

object BenchPlan {
    private const val C = 74
    private const val R = 30
    private const val WC = 200
    private const val WR = 50

    val scenarios: List<Scenario> = listOf(
        Scenario("74x30 idle", C, R, Profile.Idle),
        Scenario("74x30 mixed auto", C, R, Profile.Mixed),
        Scenario("74x30 mixed run-cache", C, R, Profile.Mixed, RenderMode.CachedRuns),
        Scenario("74x30 mixed glyph-cache", C, R, Profile.Mixed, RenderMode.PerGlyph),
        Scenario("74x30 scroll auto", C, R, Profile.Scroll),
        Scenario("74x30 mixed + 2000 scrollback", C, R, Profile.Mixed, history = 2000),
        Scenario("74x30 worst auto", C, R, Profile.Worst),
        Scenario("74x30 worst run-cache", C, R, Profile.Worst, RenderMode.CachedRuns),
        Scenario("74x30 worst glyph-cache", C, R, Profile.Worst, RenderMode.PerGlyph),
        Scenario("74x30 mixed zoom-redraw", C, R, Profile.Mixed, zoom = ZoomMode.Redraw),
        Scenario("74x30 mixed zoom-layer", C, R, Profile.Mixed, zoom = ZoomMode.Layer),
        Scenario("200x50 mixed auto", WC, WR, Profile.Mixed),
        Scenario("200x50 mixed run-cache", WC, WR, Profile.Mixed, RenderMode.CachedRuns),
        Scenario("200x50 mixed glyph-cache", WC, WR, Profile.Mixed, RenderMode.PerGlyph),
        Scenario("200x50 scroll auto", WC, WR, Profile.Scroll),
        Scenario("200x50 worst auto", WC, WR, Profile.Worst),
        Scenario("200x50 mixed zoom-redraw", WC, WR, Profile.Mixed, zoom = ZoomMode.Redraw),
        Scenario("200x50 mixed zoom-layer", WC, WR, Profile.Mixed, zoom = ZoomMode.Layer),
    )
}

class BenchResult(val scenario: Scenario, val report: Report, val hitPct: Float, val mode: RenderMode)

private fun two(x: Float): String {
    val v = (x * 100).toInt()
    return "${v / 100}.${(v % 100).toString().padStart(2, '0')}"
}

fun BenchResult.toLine(): String = "KAMPR_BENCH $platformLabel | ${scenario.name}" +
    " | frames=${report.frames}" +
    " fps=${two(report.fps)}" +
    " int_p50=${two(report.intervalP50)}" +
    " int_p95=${two(report.intervalP95)}" +
    " int_p99=${two(report.intervalP99)}" +
    " int_max=${two(report.intervalMax)}" +
    " draw_p50=${two(report.drawP50)}" +
    " draw_p95=${two(report.drawP95)}" +
    " draw_p99=${two(report.drawP99)}" +
    " model_p50=${two(report.applyP50)}" +
    " dropped=${report.dropped}" +
    " drop_pct=${two(report.dropPct)}" +
    " cache_hit_pct=${two(hitPct * 100f)}" +
    " mode=$mode"

class BenchRunner(
    private val scenarios: List<Scenario>,
    private val warmup: Int = 90,
    private val measured: Int = 300,
) {
    var index = 0
        private set
    var frame = 0
        private set
    var finished = false
        private set

    val results = ArrayList<BenchResult>()
    val current: Scenario get() = scenarios[index.coerceAtMost(scenarios.size - 1)]
    val total: Int get() = scenarios.size
    val phase: String get() = if (finished) "done" else if (frame < warmup) "warmup" else "measure"

    fun tick(
        stats: FrameStats,
        hitRate: Float,
        mode: RenderMode,
        onNext: (Scenario) -> Unit,
    ) {
        if (finished) return
        frame++
        if (frame == warmup) {
            stats.reset()
            return
        }
        if (frame < warmup + measured) return
        results.add(BenchResult(current, stats.report(BUDGET_MS), hitRate, mode))
        emitBench(results.last().toLine())
        index++
        frame = 0
        stats.reset()
        if (index >= scenarios.size) {
            finished = true
            emitBench("KAMPR_BENCH_DONE $platformLabel scenarios=${results.size}")
            return
        }
        onNext(current)
    }
}

private class Timing {
    var intervalMs = 0f
    var modelMs = 0f
}

// The bench drives the shipping GridRenderer, TextCache and ModeSelector through the shipping
// PaneState, so a number here is a number about what ships rather than about a copy of it.
@Composable
fun TerminalBenchApp() {
    KamprTheme(SoftTheme, TypeScale.Desk) { BenchBody() }
}

@Composable
private fun BenchBody() {
    val tokens = Kampr.tokens
    val palette = remember(tokens) { tokens.terminalPalette() }
    val measurer = rememberTextMeasurer(cacheSize = 0)
    val cache = remember(tokens) { TextCache(measurer, tokens.fonts.terminal) }
    val modes = remember { ModeSelector() }
    val renderer = remember(cache) { GridRenderer(cache, modes) }
    val styles = remember(palette) { ResolvedStyles(palette) }
    val pane = remember { PaneState(BENCH_PANE, StyleTable()) }
    val rows = remember(pane) { SurfaceRows(pane) }
    val workload = remember { Workload(Profile.Mixed, 74, 30) }
    val stats = remember { FrameStats() }
    val timing = remember { Timing() }
    val runner = remember { BenchRunner(BenchPlan.scenarios) }

    var tick by remember { mutableIntStateOf(0) }
    var zoom by remember { mutableStateOf(1f) }
    var scenario by remember { mutableStateOf(BenchPlan.scenarios.first()) }
    var hud by remember { mutableStateOf<Report?>(null) }
    var done by remember { mutableStateOf(false) }
    var fontEpoch by remember(cache) { mutableIntStateOf(0) }

    LaunchedEffect(cache) {
        repeat(120) {
            withFrameNanos { }
            if (cache.reprobe(BASE_CELL_SP.sp)) fontEpoch++
        }
    }

    LaunchedEffect(tokens) {
        apply(pane, BenchStyles.table())
        start(scenario, pane, workload, modes)
        var lastNanos = 0L
        var elapsed = 0.0
        var lastHud = 0.0
        var envEmitted = false
        while (true) {
            withFrameNanos { now ->
                val dt = if (lastNanos == 0L) BUDGET_MS.toDouble() else (now - lastNanos) / 1e6
                lastNanos = now
                elapsed += dt
                timing.intervalMs = dt.toFloat()

                val mark = TimeSource.Monotonic.markNow()
                for (msg in workload.step(BENCH_PANE, dt, elapsed)) apply(pane, msg)
                timing.modelMs = (mark.elapsedNow().inWholeMicroseconds / 1000.0).toFloat()

                if (scenario.zoom != ZoomMode.None) {
                    val phase = elapsed / 2500.0
                    zoom = 1.3f + 0.55f * cos(phase * 2 * PI).toFloat()
                } else {
                    zoom = 1f
                }

                tick++
                if (!envEmitted && elapsed > 3000.0) {
                    envEmitted = true
                    emitBench("KAMPR_ENV $platformLabel | ${graphicsBackend()}")
                }
                if (elapsed - lastHud > 500.0) {
                    lastHud = elapsed
                    hud = stats.report(BUDGET_MS)
                }
                runner.tick(stats, modes.hitRate, modes.mode) { next ->
                    scenario = next
                    zoom = 1f
                    start(next, pane, workload, modes)
                    renderer.reset()
                }
                if (runner.finished && !done) done = true
            }
        }
    }

    Box(Modifier.fillMaxSize().background(palette.background(pane.styles[0]))) {
        val layered = scenario.zoom == ZoomMode.Layer
        val drawZoom = if (layered) 1f else zoom
        val metrics = remember(cache, drawZoom, fontEpoch) { cache.metrics((BASE_CELL_SP * drawZoom).sp) }
        Spacer(
            Modifier
                .fillMaxSize()
                .graphicsLayer {
                    scaleX = if (layered) zoom else 1f
                    scaleY = if (layered) zoom else 1f
                    transformOrigin = TransformOrigin(0f, 0f)
                }
                .drawBehind {
                    tick
                    val mark = TimeSource.Monotonic.markNow()
                    styles.sync(pane.styles)
                    renderer.draw(
                        scope = this,
                        rows = rows,
                        styles = styles,
                        cellWidth = metrics.width,
                        cellHeight = metrics.height,
                        originX = 0f,
                        originY = size.height - rows.total * metrics.height,
                        cursorCol = pane.cursor.col,
                        cursorRow = pane.cursor.row,
                        cursorOn = pane.cursor.visible,
                        selection = null,
                        selectionWash = tokens.color.accent.copy(alpha = 0.3f),
                        linkInk = tokens.color.accentHi,
                    )
                    stats.record(
                        timing.intervalMs,
                        (mark.elapsedNow().inWholeMicroseconds / 1000.0).toFloat(),
                        timing.modelMs,
                    )
                },
        )
        Hud(hud, scenario, runner, done, modes.mode, Modifier.align(Alignment.TopStart).padding(6.dp))
    }
}

private const val PI = 3.141592653589793

private fun apply(pane: PaneState, msg: ServerMsg) {
    when (msg) {
        is ServerMsg.Styles -> pane.styles.append(msg.from, msg.styles)
        is ServerMsg.GridReset -> pane.applyReset(msg)
        is ServerMsg.GridPatch -> pane.applyPatch(msg)
        is ServerMsg.Scrollback -> pane.applyScrollback(msg)
        else -> Unit
    }
}

private fun start(scenario: Scenario, pane: PaneState, workload: Workload, modes: ModeSelector) {
    workload.reconfigure(scenario.profile, scenario.cols, scenario.rows)
    modes.forced = scenario.forced
    modes.reset()
    pane.scrollback.clear()
    pane.applyReset(workload.reset(BENCH_PANE, true))
    if (scenario.history > 0) pane.applyScrollback(workload.history(BENCH_PANE, scenario.history))
}

@Composable
private fun Hud(
    report: Report?,
    scenario: Scenario,
    runner: BenchRunner,
    done: Boolean,
    mode: RenderMode,
    modifier: Modifier,
) {
    val tokens = Kampr.tokens
    val style = TextStyle(
        fontSize = 11.sp,
        color = tokens.color.done,
        fontFamily = tokens.fonts.terminal,
    )
    Column(modifier.background(tokens.color.bar)) {
        BasicText("kampr terminal bench · $platformLabel", style = style)
        BasicText("[${runner.index + 1}/${runner.total} ${runner.phase}] ${scenario.name} · $mode", style = style)
        report?.let {
            BasicText(
                "frame p50=${two(it.intervalP50)} p95=${two(it.intervalP95)} p99=${two(it.intervalP99)}",
                style = style,
            )
            BasicText(
                "draw  p50=${two(it.drawP50)} p99=${two(it.drawP99)} model=${two(it.applyP50)}",
                style = style,
            )
            BasicText("fps=${two(it.fps)} dropped=${it.dropped} n=${it.frames}", style = style)
        }
        if (done) for (result in runner.results) {
            BasicText(
                "${result.scenario.name}: draw50=${two(result.report.drawP50)} " +
                    "drop=${result.report.dropped} hit=${two(result.hitPct * 100f)}",
                style = style,
            )
        }
    }
}
