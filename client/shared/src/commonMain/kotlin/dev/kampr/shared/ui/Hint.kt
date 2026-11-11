package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.input.pointer.PointerType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntRect
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupPositionProvider
import androidx.compose.ui.window.PopupProperties
import dev.kampr.shared.theme.Kampr
import kotlinx.coroutines.Job
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

internal const val HINT_HOVER_MS = 500L
internal const val HINT_LINGER_MS = 1500L

private val HINT_GAP = 6.dp
private val HINT_MARGIN = 8.dp

// The words an icon-only control would say if it had room: a mouse resting on it, or a finger
// held on it. Placed last inside the control's own Box, so it covers exactly the control and its
// popup is anchored to it whatever window the control lives in.
//
// It listens beside the control's clickable rather than replacing it. A held finger shows the hint
// and then eats the release on the Initial pass, which `combinedClickable` reads as a cancelled
// tap — so the press that asked what a control does never also does it. A control with a long
// press of its own keeps it, and passes `byTouch = false` to answer only a mouse.
@Composable
fun BoxScope.Hint(text: String, byTouch: Boolean = true) {
    var shown by remember { mutableStateOf(false) }
    val haptics = LocalHapticFeedback.current
    Box(
        Modifier.matchParentSize().pointerInput(byTouch) {
            coroutineScope {
                var pending: Job? = null
                var held = false
                awaitPointerEventScope {
                    while (true) {
                        val event = awaitPointerEvent(PointerEventPass.Initial)
                        val change = event.changes.firstOrNull() ?: continue
                        val touch = change.type != PointerType.Mouse
                        when (event.type) {
                            PointerEventType.Enter -> if (!touch) {
                                pending?.cancel()
                                pending = launch {
                                    delay(HINT_HOVER_MS)
                                    shown = true
                                }
                            }

                            PointerEventType.Exit -> if (!touch || !held) {
                                pending?.cancel()
                                shown = false
                            }

                            PointerEventType.Press -> {
                                pending?.cancel()
                                shown = false
                                held = false
                                if (touch && byTouch) {
                                    pending = launch {
                                        delay(viewConfiguration.longPressTimeoutMillis)
                                        held = true
                                        haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                                        shown = true
                                    }
                                }
                            }

                            PointerEventType.Move -> if (touch && !held && change.pressed) {
                                val drift = change.position - change.previousPosition
                                if (change.isConsumed || drift.getDistance() > viewConfiguration.touchSlop) pending?.cancel()
                            }

                            PointerEventType.Release -> if (touch) {
                                pending?.cancel()
                                if (held) {
                                    event.changes.forEach { it.consume() }
                                    held = false
                                    pending = launch {
                                        delay(HINT_LINGER_MS)
                                        shown = false
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
    )
    if (shown) {
        val tokens = Kampr.tokens
        val density = LocalDensity.current
        val placement = remember(density) {
            HintPlacement(with(density) { HINT_GAP.roundToPx() }, with(density) { HINT_MARGIN.roundToPx() })
        }
        Popup(popupPositionProvider = placement, properties = PopupProperties(focusable = false)) {
            val shape = RoundedCornerShape(tokens.radii.sm)
            // Inverted: every surface colour is some header's or sheet's ground, and a hint drawn in
            // one vanished into the bar it hung over.
            DisableSelection {
                KText(
                    text,
                    tokens.type.captionSmall,
                    tokens.color.bg,
                    Modifier
                        .widthIn(max = 280.dp)
                        .background(tokens.color.text, shape)
                        .padding(horizontal = 9.dp, vertical = 5.dp),
                    maxLines = 2,
                )
            }
        }
    }
}

// Above the control, where a finger holding it does not cover the words; below it when there is
// no room above, and never past either side of the window.
internal class HintPlacement(private val gap: Int, private val margin: Int) : PopupPositionProvider {
    override fun calculatePosition(anchorBounds: IntRect, windowSize: IntSize, layoutDirection: LayoutDirection, popupContentSize: IntSize): IntOffset {
        val rightmost = (windowSize.width - popupContentSize.width - margin).coerceAtLeast(margin)
        val x = (anchorBounds.center.x - popupContentSize.width / 2).coerceIn(margin, rightmost)
        val above = anchorBounds.top - gap - popupContentSize.height
        val y = if (above >= margin) above else anchorBounds.bottom + gap
        return IntOffset(x, y)
    }
}
