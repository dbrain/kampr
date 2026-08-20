package dev.kampr.terminal.spike

class Scenario(
    val name: String,
    val cols: Int,
    val rows: Int,
    val profile: Profile,
    val mode: RenderMode,
    val zoom: ZoomMode = ZoomMode.NONE,
    val recompose: Boolean = false,
)

enum class ZoomMode { NONE, REDRAW, LAYER }

object BenchPlan {
    private const val C = 74
    private const val R = 30
    private const val WC = 200
    private const val WR = 50

    val scenarios: List<Scenario> = listOf(
        Scenario("74x30 idle cursor", C, R, Profile.IDLE, RenderMode.RUN_CACHE),
        Scenario("74x30 mixed no-text", C, R, Profile.MIXED, RenderMode.NO_TEXT),
        Scenario("74x30 mixed shape-every-frame", C, R, Profile.MIXED, RenderMode.SHAPE_EVERY_FRAME),
        Scenario("74x30 mixed run-cache", C, R, Profile.MIXED, RenderMode.RUN_CACHE),
        Scenario("74x30 mixed glyph-cache", C, R, Profile.MIXED, RenderMode.GLYPH_CACHE),
        Scenario("74x30 mixed bitmap-dirty", C, R, Profile.MIXED, RenderMode.BITMAP_DIRTY),
        Scenario("74x30 mixed run-cache recompose", C, R, Profile.MIXED, RenderMode.RUN_CACHE, recompose = true),
        Scenario("74x30 scroll run-cache", C, R, Profile.SCROLL, RenderMode.RUN_CACHE),
        Scenario("74x30 worst no-text", C, R, Profile.WORST, RenderMode.NO_TEXT),
        Scenario("74x30 worst shape-every-frame", C, R, Profile.WORST, RenderMode.SHAPE_EVERY_FRAME),
        Scenario("74x30 worst run-cache", C, R, Profile.WORST, RenderMode.RUN_CACHE),
        Scenario("74x30 worst glyph-cache", C, R, Profile.WORST, RenderMode.GLYPH_CACHE),
        Scenario("74x30 mixed zoom-redraw", C, R, Profile.MIXED, RenderMode.RUN_CACHE, ZoomMode.REDRAW),
        Scenario("74x30 mixed zoom-layer", C, R, Profile.MIXED, RenderMode.RUN_CACHE, ZoomMode.LAYER),
        Scenario("200x50 mixed no-text", WC, WR, Profile.MIXED, RenderMode.NO_TEXT),
        Scenario("200x50 mixed shape-every-frame", WC, WR, Profile.MIXED, RenderMode.SHAPE_EVERY_FRAME),
        Scenario("200x50 mixed run-cache", WC, WR, Profile.MIXED, RenderMode.RUN_CACHE),
        Scenario("200x50 mixed glyph-cache", WC, WR, Profile.MIXED, RenderMode.GLYPH_CACHE),
        Scenario("200x50 mixed bitmap-dirty", WC, WR, Profile.MIXED, RenderMode.BITMAP_DIRTY),
        Scenario("200x50 scroll run-cache", WC, WR, Profile.SCROLL, RenderMode.RUN_CACHE),
        Scenario("200x50 worst run-cache", WC, WR, Profile.WORST, RenderMode.RUN_CACHE),
        Scenario("200x50 worst glyph-cache", WC, WR, Profile.WORST, RenderMode.GLYPH_CACHE),
        Scenario("200x50 mixed zoom-redraw", WC, WR, Profile.MIXED, RenderMode.RUN_CACHE, ZoomMode.REDRAW),
        Scenario("200x50 mixed zoom-layer", WC, WR, Profile.MIXED, RenderMode.RUN_CACHE, ZoomMode.LAYER),
        Scenario("74x30 mixed glyph-atlas", C, R, Profile.MIXED, RenderMode.GLYPH_ATLAS),
        Scenario("74x30 worst glyph-atlas", C, R, Profile.WORST, RenderMode.GLYPH_ATLAS),
    )
}

class BenchResult(val scenario: Scenario, val report: Report, val runCacheHitPct: Float)

fun BenchResult.toLine(): String {
    val r = report
    fun f(x: Float) = ((x * 100).toInt() / 100.0).toString()
    return "KAMPR_BENCH " + platformLabel + " | " + scenario.name +
        " | frames=" + r.frames +
        " fps=" + f(r.fps) +
        " int_p05=" + f(r.intervalP05) +
        " int_p50=" + f(r.intervalP50) +
        " int_p95=" + f(r.intervalP95) +
        " int_p99=" + f(r.intervalP99) +
        " int_max=" + f(r.intervalMax) +
        " draw_p50=" + f(r.drawP50) +
        " draw_p95=" + f(r.drawP95) +
        " draw_p99=" + f(r.drawP99) +
        " model_p50=" + f(r.applyP50) +
        " model_p95=" + f(r.applyP95) +
        " dropped=" + r.droppedFrames +
        " drop_pct=" + f(r.dropPct) +
        " cache_hit_pct=" + f(runCacheHitPct)
}

class BenchRunner(
    private val scenarios: List<Scenario>,
    private val warmupFrames: Int = 90,
    private val measureFrames: Int = 300,
) {
    var index = 0
        private set
    var frame = 0
        private set
    var finished = false
        private set

    val results = ArrayList<BenchResult>()

    val current: Scenario get() = scenarios[index.coerceAtMost(scenarios.size - 1)]

    val total get() = scenarios.size

    val phase: String
        get() = if (finished) "done" else if (frame < warmupFrames) "warmup" else "measure"

    fun tick(stats: FrameStats, budgetMs: Float, cache: TextCache, onScenarioStart: (Scenario) -> Unit): Boolean {
        if (finished) return false
        frame++
        if (frame == warmupFrames) {
            stats.reset()
            cache.resetCounters()
            return false
        }
        if (frame >= warmupFrames + measureFrames) {
            val hits = cache.runCacheHits
            val misses = cache.runCacheMisses
            val pct = if (hits + misses > 0) 100f * hits / (hits + misses) else -1f
            val res = BenchResult(current, stats.report(budgetMs), pct)
            results.add(res)
            emitBench(res.toLine())
            index++
            frame = 0
            if (index >= scenarios.size) {
                finished = true
                emitBench("KAMPR_BENCH_DONE $platformLabel scenarios=${results.size}")
                return false
            }
            stats.reset()
            cache.resetCounters()
            onScenarioStart(current)
            return true
        }
        return false
    }
}
