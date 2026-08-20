package dev.kampr.terminal.spike

class Series(private val capacity: Int) {
    private val v = FloatArray(capacity)
    private var n = 0
    private var head = 0

    fun add(x: Float) {
        v[head] = x
        head = (head + 1) % capacity
        if (n < capacity) n++
    }

    fun reset() {
        n = 0; head = 0
    }

    fun sorted(): FloatArray {
        val out = FloatArray(n)
        for (i in 0 until n) out[i] = v[(head - n + i + capacity) % capacity]
        out.sort()
        return out
    }

}

fun FloatArray.pct(p: Double): Float {
    if (isEmpty()) return 0f
    val idx = ((size - 1) * p).toInt().coerceIn(0, size - 1)
    return this[idx]
}

class Report(
    val frames: Int,
    val intervalP05: Float,
    val intervalP50: Float,
    val intervalP95: Float,
    val intervalP99: Float,
    val intervalMax: Float,
    val drawP50: Float,
    val drawP95: Float,
    val drawP99: Float,
    val applyP50: Float,
    val applyP95: Float,
    val droppedFrames: Int,
    val budgetMs: Float,
    val histogram: IntArray,
) {
    val fps: Float get() = if (intervalP50 > 0f) 1000f / intervalP50 else 0f
    val dropPct: Float get() = if (frames > 0) 100f * droppedFrames / (frames + droppedFrames) else 0f
}

class FrameStats(capacity: Int = 1200) {
    private val interval = Series(capacity)
    private val draw = Series(capacity)
    private val apply = Series(capacity)
    private var frames = 0

    fun reset() {
        interval.reset(); draw.reset(); apply.reset(); frames = 0
    }

    fun record(intervalMs: Float, drawMs: Float, applyMs: Float) {
        interval.add(intervalMs)
        draw.add(drawMs)
        apply.add(applyMs)
        frames++
    }

    fun report(budgetMs: Float): Report {
        val iv = interval.sorted()
        val dr = draw.sorted()
        val ap = apply.sorted()
        val hist = IntArray(HIST_EDGES.size + 1)
        for (x in iv) {
            var b = HIST_EDGES.size
            for (i in HIST_EDGES.indices) if (x < HIST_EDGES[i]) { b = i; break }
            hist[b]++
        }
        var dropped = 0
        for (x in iv) {
            val slots = (x / budgetMs + 0.5f).toInt()
            if (slots > 1) dropped += slots - 1
        }
        return Report(
            frames = frames,
            intervalP05 = iv.pct(0.05), intervalP50 = iv.pct(0.50),
            intervalP95 = iv.pct(0.95), intervalP99 = iv.pct(0.99),
            intervalMax = if (iv.isEmpty()) 0f else iv[iv.size - 1],
            drawP50 = dr.pct(0.50), drawP95 = dr.pct(0.95), drawP99 = dr.pct(0.99),
            applyP50 = ap.pct(0.50), applyP95 = ap.pct(0.95),
            droppedFrames = dropped,
            budgetMs = budgetMs,
            histogram = hist,
        )
    }

    companion object {
        val HIST_EDGES = floatArrayOf(4f, 8f, 12f, 16.7f, 20f, 25f, 33.4f, 50f, 100f)
        val HIST_LABELS = arrayOf(
            "<4", "4-8", "8-12", "12-16.7", "16.7-20", "20-25", "25-33", "33-50", "50-100", ">100",
        )
    }
}
