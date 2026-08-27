package dev.kampr.conversation

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.rememberTransformableState
import androidx.compose.foundation.gestures.transformable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.GlyphTarget
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LANDSCAPE_TOUCH
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.QuietAction
import dev.kampr.shared.ui.announce
import dev.kampr.shared.ui.named
import kotlin.math.max
import kotlin.math.min

private const val SMALLEST = 1f
private const val LARGEST = 8f

// Where a picture goes to be looked at rather than read past. The transcript holds a thumbnail
// because a screenshot of a wide terminal is taller than the column it lands in, and a reader who
// has to scroll past one picture to reach the next message has lost the transcript — but a
// thumbnail of a 292-column pane is unreadable, which is the whole reason this exists.
//
// A double tap resets, and is the way back from any state the gestures can reach.
class Zoomed(val zoom: Float, val panX: Float, val panY: Float)

// The load-bearing arithmetic, out where it can be checked. Getting it wrong does not look wrong —
// it strands the picture somewhere off the viewport with the gesture that would bring it back
// already spent, which is a blank screen a reader cannot argue with.
//
// Pan is bounded by the *viewport* rather than by the picture's drawn edges: half the extra width
// a zoom creates is exactly how far it can travel before its far edge arrives, which holds for any
// aspect ratio without needing the laid-out size of a `ContentScale.Fit` image. And a picture
// pulled back to its own size has nowhere left to go, so it returns to the middle rather than
// sitting in a corner the reader can no longer pan out of.
fun zoomed(was: Zoomed, scaled: Float, pannedX: Float, pannedY: Float, viewport: IntSize): Zoomed {
    val zoom = min(LARGEST, max(SMALLEST, was.zoom * scaled))
    if (zoom <= SMALLEST) return Zoomed(SMALLEST, 0f, 0f)
    val grew = zoom / was.zoom
    val roomX = (zoom - SMALLEST) * viewport.width / 2f
    val roomY = (zoom - SMALLEST) * viewport.height / 2f
    return Zoomed(
        zoom,
        min(roomX, max(-roomX, was.panX * grew + pannedX)),
        min(roomY, max(-roomY, was.panY * grew + pannedY)),
    )
}

@Composable
fun ImageViewer(
    image: ImageBitmap,
    headline: String,
    detail: String?,
    onSave: () -> Unit,
    onClose: () -> Unit,
    saved: String?,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val safe = LocalSafeArea.current
    var zoom by remember { mutableFloatStateOf(SMALLEST) }
    var panX by remember { mutableFloatStateOf(0f) }
    var panY by remember { mutableFloatStateOf(0f) }
    var viewport by remember { mutableStateOf(IntSize.Zero) }

    fun reset() {
        zoom = SMALLEST
        panX = 0f
        panY = 0f
    }

    val transform = rememberTransformableState { scaled, panned, _ ->
        val next = zoomed(Zoomed(zoom, panX, panY), scaled, panned.x, panned.y, viewport)
        zoom = next.zoom
        panX = next.panX
        panY = next.panY
    }

    Box(
        modifier
            .fillMaxSize()
            .background(tokens.color.bg)
            .named("$headline, pinch to zoom, double tap to fit"),
    ) {
        Image(
            bitmap = image,
            contentDescription = null,
            modifier = Modifier
                .fillMaxSize()
                .padding(top = 56.dp + safe.top, bottom = 64.dp + safe.bottom)
                .onSizeChanged { viewport = it }
                .graphicsLayer {
                    scaleX = zoom
                    scaleY = zoom
                    translationX = panX
                    translationY = panY
                }
                .transformable(transform)
                .pointerInput(Unit) { detectTapGestures(onDoubleTap = { reset() }) },
            contentScale = ContentScale.Fit,
        )
        DisableSelection {
            Row(
                Modifier
                    .fillMaxWidth()
                    .align(Alignment.TopStart)
                    .background(tokens.color.bar)
                    .padding(
                        start = 16.dp + safe.left,
                        end = 16.dp + safe.right,
                        top = 10.dp + safe.top,
                        bottom = 10.dp,
                    ),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Column(Modifier.weight(1f)) {
                    KText(headline, tokens.type.meta, tokens.color.text)
                    detail?.let { KText(it, tokens.type.micro, tokens.color.mute) }
                }
                GlyphTarget(
                    ConversationIcons.close, "Close $headline", tokens.color.mute,
                    onClose, target = LANDSCAPE_TOUCH, glyph = 14.dp,
                )
            }
            Column(
                Modifier
                    .fillMaxWidth()
                    .align(Alignment.BottomStart)
                    .background(tokens.color.bar)
                    .padding(
                        start = 16.dp + safe.left,
                        end = 16.dp + safe.right,
                        top = 10.dp,
                        bottom = 12.dp + safe.bottom,
                    ),
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                if (saved == null) {
                    QuietAction(
                        "Save to device",
                        onSave,
                        Modifier.fillMaxWidth(),
                        label = "Save $headline to this device",
                    )
                } else {
                    KText(
                        "saved to $saved",
                        tokens.type.micro,
                        tokens.color.done,
                        Modifier.fillMaxWidth().announce("Saved to $saved"),
                        maxLines = 2,
                    )
                }
            }
        }
    }
}
