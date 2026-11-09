package dev.kampr.terminal.view

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import dev.kampr.shared.ui.Track
import kotlin.math.ln
import kotlin.math.pow

// Tagged so a test can assert where the thumb actually *is*. Asserting only that a drag moves the
// zoom passes on a thumb pinned to the left end, which is what shipped until a browser showed it.
internal const val THUMB_TAG = "zoom-slider-thumb"

// Zoom is multiplicative — every other control here multiplies it, and the presets are spaced by
// ratio rather than by difference — so the track is logarithmic. A linear one spends most of its
// length above `readable` and leaves the whole usable range crushed into the first few millimetres.
internal fun zoomFraction(zoom: Float, presets: ZoomPresets): Float {
    val lo = presets.minimum
    val hi = presets.maximum
    if (hi <= lo || zoom <= 0f) return 0f
    return (ln(zoom / lo) / ln(hi / lo)).coerceIn(0f, 1f)
}

internal fun zoomAt(fraction: Float, presets: ZoomPresets): Float {
    val lo = presets.minimum
    val hi = presets.maximum
    if (hi <= lo) return lo
    return lo * (hi / lo).pow(fraction.coerceIn(0f, 1f))
}

@Composable
internal fun ZoomSlider(
    presets: ZoomPresets,
    zoom: Float,
    onZoom: (Float) -> Unit,
    modifier: Modifier = Modifier,
) {
    Track(
        zoomFraction(zoom, presets),
        { onZoom(zoomAt(it, presets)) },
        "Zoom level, ${zoomLabel(zoom)}",
        zoomLabel(zoom),
        modifier,
        thumbTag = THUMB_TAG,
    )
}
