package dev.kampr.conversation.files

import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import dev.kampr.shared.platform.LocalReduceMotion
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.announce

const val SKELETON_TAG = "files-skeleton"

private val ROW_WIDTHS = listOf(0.62f, 0.44f, 0.71f, 0.38f, 0.55f, 0.49f)
private val LINE_WIDTHS = listOf(0.82f, 0.64f, 0.9f, 0.35f, 0.72f, 0.58f, 0.86f, 0.41f, 0.67f, 0.77f)

enum class SkeletonShape { Rows, Lines }

private const val PULSE_MS = 900

@Composable
fun Skeleton(shape: SkeletonShape, what: String, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val breathing = if (LocalReduceMotion.current) {
        Modifier
    } else {
        val pulse = rememberInfiniteTransition("skeleton").animateFloat(
            initialValue = 0.4f,
            targetValue = 1f,
            animationSpec = infiniteRepeatable(tween(PULSE_MS, easing = FastOutSlowInEasing), RepeatMode.Reverse),
            label = "pulse",
        )
        Modifier.graphicsLayer { alpha = pulse.value }
    }
    val fill = Modifier.background(tokens.color.raise, RoundedCornerShape(tokens.radii.sm))
    Column(
        modifier.fillMaxWidth().padding(16.dp).announce("Loading $what").testTag(SKELETON_TAG).then(breathing),
        verticalArrangement = Arrangement.spacedBy(if (shape == SkeletonShape.Rows) 18.dp else 10.dp),
    ) {
        when (shape) {
            SkeletonShape.Rows -> ROW_WIDTHS.forEach { width ->
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    Column(Modifier.size(16.dp).then(fill)) {}
                    Column(Modifier.fillMaxWidth(width).height(12.dp).then(fill)) {}
                }
            }

            SkeletonShape.Lines -> LINE_WIDTHS.forEach { width ->
                Column(Modifier.fillMaxWidth(width).height(10.dp).then(fill)) {}
            }
        }
    }
}
