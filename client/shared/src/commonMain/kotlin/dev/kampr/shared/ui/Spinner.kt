package dev.kampr.shared.ui

import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr

@Composable
fun Spinner(label: String, size: Dp = 22.dp, modifier: Modifier = Modifier) {
    val tint = Kampr.tokens.color.mute
    val turn = rememberInfiniteTransition("spinner").animateFloat(
        0f,
        360f,
        infiniteRepeatable(tween(900, easing = LinearEasing), RepeatMode.Restart),
        "turn",
    )
    Canvas(modifier.size(size).announce(label)) {
        val width = 2.dp.toPx()
        drawArc(tint.copy(alpha = 0.25f), 0f, 360f, false, style = Stroke(width))
        drawArc(tint, turn.value, 90f, false, style = Stroke(width, cap = StrokeCap.Round))
    }
}

// A state said in its own colour, small enough to sit beside a title.
@Composable
fun StateChip(text: String, tone: Color, modifier: Modifier = Modifier) {
    val shape = RoundedCornerShape(Kampr.tokens.radii.pill)
    Box(modifier.background(tone.copy(alpha = 0.16f), shape).padding(horizontal = 8.dp, vertical = 2.dp)) {
        DisableSelection { KText(text, Kampr.tokens.type.badge, tone) }
    }
}
