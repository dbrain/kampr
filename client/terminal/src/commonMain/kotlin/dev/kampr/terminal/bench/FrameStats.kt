package dev.kampr.terminal.bench

class Series(private val capacity: Int) {
    private val values = FloatArray(capacity)
    private var count = 0
    private var head = 0

    fun add(value: Float) {
        values[head] = value
        head = (head + 1) % capacity
        if (count < capacity) count++
    }

    fun reset() {
        count = 0
        head = 0
    }

    fun sorted(): FloatArray {
        val out = FloatArray(count)
        for (i in 0 until count) out[i] = values[(head - count + i + capacity) % capacity]
        out.sort()
        return out
    }
}

private fun FloatArray.pct(p: Double): Float {
    if (isEmpty()) return 0f
    return this[(((size - 1) * p).toInt()).coerceIn(0, size - 1)]
}

class Report(
    val frames: Int,
    val intervalP50: Float,
    val intervalP95: Float,
    val intervalP99: Float,
    val intervalMax: Float,
    val drawP50: Float,
    val drawP95: Float,
    val drawP99: Float,
    val applyP50: Float,
    val dropped: Int,
) {
    val fps: Float get() = if (intervalP50 > 0f) 1000f / intervalP50 else 0f
    val dropPct: Float get() = if (frames > 0) 100f * dropped / (frames + dropped) else 0f
}

class FrameStats(capacity: Int = 1200) {
    private val interval = Series(capacity)
    private val draw = Series(capacity)
    private val apply = Series(capacity)
    private var frames = 0

    fun reset() {
        interval.reset()
        draw.reset()
        apply.reset()
        frames = 0
    }

    fun record(intervalMs: Float, drawMs: Float, applyMs: Float) {
        interval.add(intervalMs)
        draw.add(drawMs)
        apply.add(applyMs)
        frames++
    }

    // Dropped frames are counted as vsync slots skipped, the same way the spike counted them.
    fun report(budgetMs: Float): Report {
        val intervals = interval.sorted()
        val draws = draw.sorted()
        val applies = apply.sorted()
        var dropped = 0
        for (value in intervals) {
            val slots = (value / budgetMs + 0.5f).toInt()
            if (slots > 1) dropped += slots - 1
        }
        return Report(
            frames = frames,
            intervalP50 = intervals.pct(0.50),
            intervalP95 = intervals.pct(0.95),
            intervalP99 = intervals.pct(0.99),
            intervalMax = if (intervals.isEmpty()) 0f else intervals[intervals.size - 1],
            drawP50 = draws.pct(0.50),
            drawP95 = draws.pct(0.95),
            drawP99 = draws.pct(0.99),
            applyP50 = applies.pct(0.50),
            dropped = dropped,
        )
    }
}
