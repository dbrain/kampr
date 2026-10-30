package dev.kampr.conversation.files

import androidx.compose.foundation.Image
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculateCentroid
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChanged
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.IntSize
import dev.kampr.shared.ui.gestureAction
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt

private const val SMALLEST = 1f
private const val LARGEST = 8f
const val DOUBLE_TAP_ZOOM = 2.5f
private const val WHEEL_STEP = 1.15f

class Zoomed(val zoom: Float, val panX: Float, val panY: Float)

// Getting this wrong does not look wrong: it strands the picture off the viewport with the gesture
// that would bring it back already spent. Pan is bounded by the viewport rather than the drawn
// edges — half the extra width a zoom creates is how far it can travel, for any aspect ratio,
// without knowing the laid-out size of a `ContentScale.Fit` image.
//
// `focusX`/`focusY` are where the fingers are, from the viewport's centre: the picture under them
// stays under them.
fun zoomed(
    was: Zoomed,
    scaled: Float,
    pannedX: Float,
    pannedY: Float,
    viewport: IntSize,
    focusX: Float = 0f,
    focusY: Float = 0f,
): Zoomed {
    val zoom = min(LARGEST, max(SMALLEST, was.zoom * scaled))
    if (zoom <= SMALLEST) return Zoomed(SMALLEST, 0f, 0f)
    val grew = zoom / was.zoom
    val roomX = (zoom - SMALLEST) * viewport.width / 2f
    val roomY = (zoom - SMALLEST) * viewport.height / 2f
    return Zoomed(
        zoom,
        min(roomX, max(-roomX, was.panX * grew + focusX * (1f - grew) + pannedX)),
        min(roomY, max(-roomY, was.panY * grew + focusY * (1f - grew) + pannedY)),
    )
}

fun doubleTapped(was: Zoomed, focusX: Float, focusY: Float, viewport: IntSize): Zoomed = if (was.zoom > SMALLEST) {
    Zoomed(SMALLEST, 0f, 0f)
} else {
    zoomed(was, DOUBLE_TAP_ZOOM / was.zoom, 0f, 0f, viewport, focusX, focusY)
}

// The gestures sit on an untransformed box and the picture is scaled inside it. Detected on the
// scaled layer itself, every delta arrives divided by the zoom it is changing, so a pinch slows
// as it works and a pan at 4x moves a quarter of the finger. Two fingers are consumed outright,
// so nothing above the viewer reads a pinch as its own scroll.
@Composable
fun ZoomImage(image: ImageBitmap, label: String, modifier: Modifier = Modifier) {
    var at by remember(image) { mutableStateOf(Zoomed(SMALLEST, 0f, 0f)) }
    var viewport by remember { mutableStateOf(IntSize.Zero) }

    fun centred(p: Offset) = Offset(p.x - viewport.width / 2f, p.y - viewport.height / 2f)

    Box(
        modifier
            .fillMaxSize()
            .clipToBounds()
            .onSizeChanged { viewport = it }
            .gestureAction(
                label = "$label, pinch to zoom, double tap to fit",
                onClick = { at = Zoomed(SMALLEST, 0f, 0f) },
                role = Role.Image,
                state = if (at.zoom <= SMALLEST) "fitted" else "zoomed ${(at.zoom * 100).roundToInt()}%",
                clickLabel = "Fit to the screen",
            )
            .pointerInput(image) {
                detectTapGestures(onDoubleTap = { tap ->
                    val f = centred(tap)
                    at = doubleTapped(at, f.x, f.y, viewport)
                })
            }
            .pointerInput(image) {
                val slop = viewConfiguration.touchSlop
                awaitEachGesture {
                    awaitFirstDown(requireUnconsumed = false)
                    var dragged = 0f
                    var multi = false
                    do {
                        val event = awaitPointerEvent()
                        val pressed = event.changes.count { it.pressed }
                        if (pressed >= 2) multi = true
                        val pan = event.calculatePan()
                        val scale = event.calculateZoom()
                        if (!multi) dragged += abs(pan.x) + abs(pan.y)
                        val moving = multi || (at.zoom > SMALLEST && dragged > slop)
                        if (moving && (scale != 1f || pan != Offset.Zero)) {
                            val f = centred(event.calculateCentroid(useCurrent = true))
                            at = zoomed(at, scale, pan.x, pan.y, viewport, f.x, f.y)
                        }
                        if (moving) event.changes.forEach { if (it.positionChanged()) it.consume() }
                    } while (event.changes.any { it.pressed })
                }
            }
            .pointerInput(image) {
                awaitPointerEventScope {
                    while (true) {
                        val event = awaitPointerEvent(PointerEventPass.Main)
                        if (event.type != PointerEventType.Scroll) continue
                        val change = event.changes.firstOrNull() ?: continue
                        val dy = change.scrollDelta.y
                        if (dy == 0f) continue
                        val f = centred(change.position)
                        at = zoomed(at, if (dy < 0) WHEEL_STEP else 1f / WHEEL_STEP, 0f, 0f, viewport, f.x, f.y)
                        change.consume()
                    }
                }
            },
    ) {
        Image(
            bitmap = image,
            contentDescription = null,
            modifier = Modifier
                .fillMaxSize()
                .graphicsLayer {
                    scaleX = at.zoom
                    scaleY = at.zoom
                    translationX = at.panX
                    translationY = at.panY
                },
            contentScale = ContentScale.Fit,
        )
    }
}
