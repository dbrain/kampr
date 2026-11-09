package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.layout
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.setProgress
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr

private val TRACK = 4.dp
private val THUMB = 22.dp

// Built out of `compose-foundation` because there is no Material on this classpath to take a
// `Slider` from. The track is `ColumnIndicator`'s fractional placement and the thumb is
// `SelectionLayer`'s, which are the only two things in this client shaped like either.
//
// `onFraction` is every step of a drag; `onSettled` is the hand lifting, for a value that costs
// something to apply — a seek is a range request, and one per pixel of drag is a flood.
@Composable
fun Track(
    fraction: Float,
    onFraction: (Float) -> Unit,
    spoken: String,
    reading: String,
    modifier: Modifier = Modifier,
    onSettled: () -> Unit = {},
    jumps: Boolean = false,
    thumbTag: String? = null,
) {
    val tokens = Kampr.tokens
    var width by remember { mutableFloatStateOf(0f) }
    val thumbPx = with(LocalDensity.current) { THUMB.toPx() }
    val at by rememberUpdatedState(fraction.coerceIn(0f, 1f))
    val step by rememberUpdatedState(onFraction)
    val settle by rememberUpdatedState(onSettled)

    val drag = rememberDraggableState { delta ->
        if (width > 0f) step((at + delta / width).coerceIn(0f, 1f))
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
            .draggable(drag, Orientation.Horizontal, onDragStopped = { settle() })
            .let { base ->
                if (!jumps) {
                    base
                } else {
                    base.pointerInput(Unit) {
                        detectTapGestures { tap ->
                            if (width > 0f) {
                                step((tap.x / width).coerceIn(0f, 1f))
                                settle()
                            }
                        }
                    }
                }
            }
            // `Modifier.action` models a click and nothing else, so a hand-built range control has
            // to carry its own reading or a screen reader is told only that something is there.
            .semantics(mergeDescendants = true) {
                contentDescription = spoken
                stateDescription = reading
                progressBarRangeInfo = ProgressBarRangeInfo(at, 0f..1f)
                setProgress { target ->
                    step(target.coerceIn(0f, 1f))
                    settle()
                    true
                }
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
        // at the left end whatever the value was — which is exactly how it looked in a browser.
        Box(
            Modifier
                .offset { IntOffset(((width - thumbPx) * at).toInt(), 0) }
                .let { if (thumbTag != null) it.testTag(thumbTag) else it }
                .size(THUMB)
                .background(tokens.color.accent, CircleShape),
        )
    }
}
