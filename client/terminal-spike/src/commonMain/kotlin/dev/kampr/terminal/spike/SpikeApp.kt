package dev.kampr.terminal.spike

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.text.BasicText
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

import dev.kampr.terminal.spike.res.Res
import dev.kampr.terminal.spike.res.jetbrainsmononl_bold
import dev.kampr.terminal.spike.res.jetbrainsmononl_bolditalic
import dev.kampr.terminal.spike.res.jetbrainsmononl_italic
import dev.kampr.terminal.spike.res.jetbrainsmononl_regular
import kotlin.math.cos
import kotlin.math.sin
import kotlin.time.TimeSource
import org.jetbrains.compose.resources.Font
import org.jetbrains.compose.resources.preloadFont

private const val BUDGET_MS = 1000f / 60f

private class FrameTiming {
    var intervalMs = 0f
    var modelMs = 0f
}

@Composable
fun TerminalSpikeApp() {
    val regular by preloadFont(Res.font.jetbrainsmononl_regular)
    val bold by preloadFont(Res.font.jetbrainsmononl_bold, FontWeight.Bold, FontStyle.Normal)
    val italic by preloadFont(Res.font.jetbrainsmononl_italic, FontWeight.Normal, FontStyle.Italic)
    val boldItalic by preloadFont(Res.font.jetbrainsmononl_bolditalic, FontWeight.Bold, FontStyle.Italic)
    val loaded = regular != null && bold != null && italic != null && boldItalic != null
    if (!loaded) {
        Box(Modifier.fillMaxSize().background(Color(Palette.DEFAULT_BG)))
        return
    }
    TerminalSpikeBench(FontFamily(regular!!, bold!!, italic!!, boldItalic!!))
}

@Composable
private fun TerminalSpikeBench(family: FontFamily) {
    val measurer = rememberTextMeasurer(cacheSize = 0)
    val cache = remember(family) { TextCache(measurer, family) }
    val renderer = remember(cache) { GridRenderer(cache) }

    val buffer = remember { CellBuffer(74, 30) }
    val styles = remember { StyleTable().apply { append(SpikeStyles.table()) } }
    val workload = remember { Workload(Profile.MIXED, 74, 30) }
    val stats = remember { FrameStats() }
    val timing = remember { FrameTiming() }
    val runner = remember { BenchRunner(BenchPlan.scenarios) }

    val tick = remember { mutableIntStateOf(0) }
    val zoom = remember { mutableFloatStateOf(1f) }
    val panX = remember { mutableFloatStateOf(0f) }
    val panY = remember { mutableFloatStateOf(0f) }

    var scenario by remember { mutableStateOf(BenchPlan.scenarios.first()) }
    var hud by remember { mutableStateOf<Report?>(null) }
    var done by remember { mutableStateOf(false) }
    var cursorOn by remember { mutableStateOf(true) }

    val fit = remember(cache) { FitState() }

    LaunchedEffect(family) {
        applyScenario(scenario, buffer, workload, renderer)
        var lastNanos = 0L
        var lastHud = 0.0
        var elapsedMs = 0.0
        var envEmitted = false
        while (true) {
            withFrameNanos { now ->
                val dt = if (lastNanos == 0L) BUDGET_MS.toDouble() else (now - lastNanos) / 1e6
                lastNanos = now
                elapsedMs += dt
                timing.intervalMs = dt.toFloat()

                val t0 = TimeSource.Monotonic.markNow()
                for (msg in workload.step(dt, elapsedMs)) {
                    when (msg) {
                        is GridReset -> buffer.apply(msg)
                        is GridPatch -> buffer.apply(msg)
                        is StylesMsg -> styles.append(msg)
                    }
                }
                timing.modelMs = (t0.elapsedNow().inWholeMicroseconds / 1000.0).toFloat()
                cursorOn = ((elapsedMs / 530.0).toInt() % 2) == 0

                when (scenario.zoom) {
                    ZoomMode.NONE -> Unit
                    else -> {
                        val phase = elapsedMs / 2500.0
                        zoom.floatValue = (1.3f + 0.55f * cos(phase * 2 * PI_F).toFloat())
                        panX.floatValue = 40f * sin(phase * 2 * PI_F).toFloat()
                        panY.floatValue = 25f * cos(phase * 3 * PI_F).toFloat()
                    }
                }

                tick.intValue++
                if (!envEmitted && elapsedMs > 3000.0) {
                    envEmitted = true
                    emitBench("KAMPR_ENV $platformLabel | ${graphicsBackend()}")
                }

                if (elapsedMs - lastHud > 500.0) {
                    lastHud = elapsedMs
                    hud = stats.report(BUDGET_MS)
                }
                runner.tick(stats, BUDGET_MS, cache) { next ->
                    scenario = next
                    zoom.floatValue = 1f
                    panX.floatValue = 0f
                    panY.floatValue = 0f
                    applyScenario(next, buffer, workload, renderer)
                }
                if (runner.finished && !done) done = true
            }
        }
    }

    Box(Modifier.fillMaxSize().background(Color(Palette.DEFAULT_BG))) {
        GridSurface(
            buffer = buffer,
            styles = styles,
            cache = cache,
            renderer = renderer,
            stats = stats,
            timing = timing,
            tick = tick,
            zoom = zoom,
            panX = panX,
            panY = panY,
            scenario = scenario,
            cursorOn = cursorOn,
            fit = fit,
        )
        Hud(
            report = hud,
            scenario = scenario,
            runner = runner,
            done = done,
            modifier = Modifier.align(Alignment.TopStart).padding(6.dp),
        )
    }
}

private const val PI_F = 3.1415927

private fun applyScenario(
    s: Scenario,
    buffer: CellBuffer,
    workload: Workload,
    renderer: GridRenderer,
) {
    buffer.resize(s.cols, s.rows)
    buffer.markAllDirty()
    workload.reconfigure(s.profile, s.cols, s.rows)
    renderer.invalidate()
}

@Composable
private fun GridSurface(
    buffer: CellBuffer,
    styles: StyleTable,
    cache: TextCache,
    renderer: GridRenderer,
    stats: FrameStats,
    timing: FrameTiming,
    tick: androidx.compose.runtime.MutableIntState,
    zoom: androidx.compose.runtime.MutableFloatState,
    panX: androidx.compose.runtime.MutableFloatState,
    panY: androidx.compose.runtime.MutableFloatState,
    scenario: Scenario,
    cursorOn: Boolean,
    fit: FitState,
) {
    val forced = if (scenario.recompose) tick.intValue else 0
    val layerZoom = scenario.zoom == ZoomMode.LAYER

    var m = Modifier.fillMaxSize()
    if (layerZoom) {
        m = m.graphicsLayer {
            scaleX = zoom.floatValue
            scaleY = zoom.floatValue
            translationX = panX.floatValue
            translationY = panY.floatValue
            transformOrigin = TransformOrigin(0f, 0f)
        }
    }
    m = m.pointerInput(scenario) {
        detectTransformGestures { _, pan, gestureZoom, _ ->
            if (scenario.zoom == ZoomMode.NONE) {
                zoom.floatValue = (zoom.floatValue * gestureZoom).coerceIn(0.3f, 6f)
                panX.floatValue += pan.x
                panY.floatValue += pan.y
            }
        }
    }
    m = m.drawBehind {
        @Suppress("UNUSED_EXPRESSION")
        forced
        tick.intValue
        val start = TimeSource.Monotonic.markNow()

        val base = fit.fontSizeSp(cache, buffer.cols, buffer.rows, size.width, size.height)
        val sizeSp = if (layerZoom) base else base * zoom.floatValue
        val metrics = cache.metricsFor(sizeSp)
        val ox = if (layerZoom) 2f else panX.floatValue + 2f
        val oy = if (layerZoom) 2f else panY.floatValue + 2f

        renderer.render(this, buffer, styles, metrics, scenario.mode, ox, oy, cursorOn)

        val drawMs = (start.elapsedNow().inWholeMicroseconds / 1000.0).toFloat()
        stats.record(timing.intervalMs, drawMs, timing.modelMs)
    }
    Spacer(m)
}

private class FitState {
    private var key = 0L
    private var value = 12f

    fun fontSizeSp(cache: TextCache, cols: Int, rows: Int, wPx: Float, hPx: Float): TextUnit {
        val k = (cols.toLong() shl 40) xor (rows.toLong() shl 24) xor
            (wPx.toInt().toLong() shl 12) xor hPx.toInt().toLong()
        if (k != key) {
            val ref = 40f
            val refM = cache.metrics(ref.sp)
            val sw = (wPx - 6f) / (cols * refM.cellW)
            val sh = (hPx - 6f) / (rows * refM.cellH)
            value = (ref * minOf(sw, sh)).coerceIn(3f, 40f)
            key = k
        }
        return value.sp
    }
}

@Composable
private fun Hud(
    report: Report?,
    scenario: Scenario,
    runner: BenchRunner,
    done: Boolean,
    modifier: Modifier,
) {
    val label = TextStyle(fontSize = 11.sp, color = Color(0xFF9AE6B4), fontFamily = FontFamily.Monospace)
    Column(modifier.background(Color(0xC0000000))) {
        BasicText("kampr terminal spike · $platformLabel", style = label)
        BasicText(
            "[${runner.index + 1}/${runner.total} ${runner.phase}] ${scenario.name}",
            style = label,
        )
        if (report != null) {
            BasicText(
                "frame ms p50=${fmt(report.intervalP50)} p95=${fmt(report.intervalP95)} " +
                    "p99=${fmt(report.intervalP99)} max=${fmt(report.intervalMax)}",
                style = label,
            )
            BasicText(
                "draw  ms p50=${fmt(report.drawP50)} p95=${fmt(report.drawP95)} " +
                    "p99=${fmt(report.drawP99)}  model p50=${fmt(report.applyP50)}",
                style = label,
            )
            BasicText(
                "fps=${fmt(report.fps)} dropped=${report.droppedFrames} " +
                    "(${fmt(report.dropPct)}%) n=${report.frames}",
                style = label,
            )
            BasicText(histogramLine(report), style = label)
        }
        if (done) {
            BasicText("— results —", style = label)
            for (r in runner.results) {
                BasicText(
                    "${r.scenario.name}: p50=${fmt(r.report.intervalP50)} " +
                        "p95=${fmt(r.report.intervalP95)} p99=${fmt(r.report.intervalP99)} " +
                        "draw50=${fmt(r.report.drawP50)} drop=${r.report.droppedFrames}",
                    style = label,
                )
            }
        }
    }
}

private fun histogramLine(r: Report): String {
    val total = r.histogram.sum().coerceAtLeast(1)
    val sb = StringBuilder()
    for (i in r.histogram.indices) {
        if (r.histogram[i] == 0) continue
        if (sb.isNotEmpty()) sb.append("  ")
        sb.append(FrameStats.HIST_LABELS[i]).append(':')
        sb.append(100 * r.histogram[i] / total).append('%')
    }
    return "hist " + sb.toString()
}

private fun fmt(x: Float): String {
    val v = (x * 100).toInt()
    return "${v / 100}.${(v % 100).toString().padStart(2, '0')}"
}
