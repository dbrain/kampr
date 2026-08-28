package dev.kampr.terminal.view

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.layout
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.IntOffset
import androidx.compose.foundation.layout.offset
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.touchable
import kotlin.math.ln
import kotlin.math.pow

// Tagged so a test can assert where the thumb actually *is*. Asserting only that a drag moves the
// zoom passes on a thumb pinned to the left end, which is what shipped until a browser showed it.
internal const val THUMB_TAG = "zoom-slider-thumb"

private val TRACK = 4.dp
private val THUMB = 22.dp

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

// Built out of `compose-foundation` because there is no Material on this classpath to take a
// `Slider` from. The track is `ColumnIndicator`'s fractional placement and the thumb is
// `SelectionLayer`'s, which are the only two things in this client shaped like either.
@Composable
internal fun ZoomSlider(
    presets: ZoomPresets,
    zoom: Float,
    onZoom: (Float) -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    var width by remember { mutableFloatStateOf(0f) }
    val thumbPx = with(LocalDensity.current) { THUMB.toPx() }
    val fraction = zoomFraction(zoom, presets)
    val spoken = "Zoom level, ${zoomLabel(zoom)}"

    val drag = rememberDraggableState { delta ->
        if (width > 0f) onZoom(zoomAt(fraction + delta / width, presets))
    }

    Box(
        modifier
            .fillMaxWidth()
            .touchable()
            .layout { measurable, constraints ->
                val placeable = measurable.measure(constraints)
                width = constraints.maxWidth.toFloat()
                layout(placeable.width, placeable.height) { placeable.place(0, 0) }
            }
            .draggable(drag, Orientation.Horizontal)
            // `Modifier.action` models a click and nothing else, so a hand-built range control has
            // to carry its own reading or a screen reader is told only that something is there.
            .semantics(mergeDescendants = true) {
                contentDescription = spoken
                stateDescription = zoomLabel(zoom)
                progressBarRangeInfo = ProgressBarRangeInfo(fraction, 0f..1f)
            },
    ) {
        Box(
            Modifier
                .fillMaxWidth()
                .padding(horizontal = THUMB / 2)
                .height(TRACK)
                .background(tokens.color.raise, RoundedCornerShape(TRACK / 2)),
        )
        // Offset, not a `layout` block. A `layout` after `.size(THUMB)` is handed the *thumb's*
        // constraints rather than the track's, so its travel was `22.dp - 22.dp` and the thumb sat
        // at the left end whatever the zoom was — which is exactly how it looked in a browser.
        Box(
            Modifier
                .offset { IntOffset(((width - thumbPx) * fraction).toInt(), 0) }
                .testTag(THUMB_TAG)
                .size(THUMB)
                .background(tokens.color.accent, CircleShape),
        )
    }
}
