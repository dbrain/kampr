package dev.kampr.terminal.render

enum class RenderMode { CachedRuns, PerGlyph }

// Probe #62: cached runs are 4x cheaper on real pane output but collapse to 30 fps when the run
// set churns; per-glyph holds 60 fps there because the glyph set is bounded and the run set is not.
// So the mode follows the measured hit rate, with hysteresis so it cannot flap frame to frame.
class ModeSelector(
    private val dropTo: Float = 0.70f,
    private val riseTo: Float = 0.90f,
    private val window: Int = 8,
) {
    var forced: RenderMode? = null

    var measured = RenderMode.CachedRuns
        private set

    val mode: RenderMode get() = forced ?: measured

    private var smoothed = 1f
    private var samples = 0

    private var previousRunKeys = HashSet<Int>()
    private var currentRunKeys = HashSet<Int>()

    fun observeRunKey(hash: Int) {
        currentRunKeys.add(hash)
    }

    // In per-glyph mode nothing populates the run cache, so its hit rate can never recover on its
    // own. Frame-to-frame overlap of the run set is the signal that the churn has stopped, and it
    // is a lower bound on what the cache would have achieved.
    private fun overlap(): Float {
        if (currentRunKeys.isEmpty()) return 1f
        var shared = 0
        for (key in currentRunKeys) if (key in previousRunKeys) shared++
        return shared.toFloat() / currentRunKeys.size
    }

    fun endFrame(cacheHitRate: Float) {
        val sample = if (mode == RenderMode.CachedRuns) cacheHitRate else overlap()
        smoothed = if (samples == 0) sample else smoothed + (sample - smoothed) / window
        samples++
        val swap = previousRunKeys
        previousRunKeys = currentRunKeys
        swap.clear()
        currentRunKeys = swap
        if (samples < window / 2) return
        measured = when {
            mode == RenderMode.CachedRuns && smoothed < dropTo -> RenderMode.PerGlyph
            mode == RenderMode.PerGlyph && smoothed > riseTo -> RenderMode.CachedRuns
            else -> measured
        }
    }

    val hitRate: Float get() = smoothed

    fun reset() {
        measured = RenderMode.CachedRuns
        smoothed = 1f
        samples = 0
        previousRunKeys.clear()
        currentRunKeys.clear()
    }
}
