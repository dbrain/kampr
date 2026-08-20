package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicText
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.BorderSpec
import dev.kampr.shared.theme.Kampr

@Composable
fun KText(
    text: String,
    style: TextStyle,
    color: Color,
    modifier: Modifier = Modifier,
    maxLines: Int = 1,
) {
    BasicText(
        text = text,
        modifier = modifier,
        style = style.copy(color = color),
        maxLines = maxLines,
        overflow = TextOverflow.Ellipsis,
    )
}

@Composable
fun LabelText(text: String, style: TextStyle, color: Color, modifier: Modifier = Modifier) {
    val label = Kampr.tokens.label
    KText(if (label.uppercase) text.uppercase() else text, style, color, modifier)
}

fun Modifier.edge(spec: BorderSpec, shape: Shape): Modifier =
    if (spec.visible) border(spec.width, spec.color, shape) else this

private fun Modifier.chromeEdge(spec: BorderSpec, side: Side): Modifier {
    if (!spec.visible) return this
    return drawBehind {
        val thickness = spec.width.toPx()
        val rect = when (side) {
            Side.Top -> Offset.Zero to Size(size.width, thickness)
            Side.Bottom -> Offset(0f, size.height - thickness) to Size(size.width, thickness)
            Side.Start -> Offset.Zero to Size(thickness, size.height)
            Side.End -> Offset(size.width - thickness, 0f) to Size(thickness, size.height)
        }
        drawRect(spec.color, topLeft = rect.first, size = rect.second)
    }
}

private enum class Side { Top, Bottom, Start, End }

@Composable
fun Modifier.edgeTop(spec: BorderSpec = Kampr.tokens.chrome): Modifier = chromeEdge(spec, Side.Top)

@Composable
fun Modifier.edgeBottom(spec: BorderSpec = Kampr.tokens.chrome): Modifier = chromeEdge(spec, Side.Bottom)

@Composable
fun Modifier.edgeEnd(spec: BorderSpec = Kampr.tokens.chrome): Modifier = chromeEdge(spec, Side.End)

@Composable
fun Surface(
    modifier: Modifier = Modifier,
    background: Color = Kampr.tokens.color.surface,
    radius: Dp = Kampr.tokens.radii.lg,
    border: BorderSpec = Kampr.tokens.card,
    content: @Composable () -> Unit,
) {
    val shape = RoundedCornerShape(radius)
    Box(modifier.background(background, shape).edge(border, shape)) { content() }
}

@Composable
fun Pill(
    modifier: Modifier = Modifier,
    background: Color = Kampr.tokens.color.surface,
    border: BorderSpec = Kampr.tokens.card,
    horizontal: Dp = 12.dp,
    vertical: Dp = 5.dp,
    content: @Composable RowScope.() -> Unit,
) {
    val shape = RoundedCornerShape(Kampr.tokens.radii.pill)
    Row(
        modifier
            .background(background, shape)
            .edge(border, shape)
            .padding(horizontal = horizontal, vertical = vertical),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(7.dp),
        content = content,
    )
}

@Composable
fun Dot(color: Color, size: Dp = 8.dp, hollow: Boolean = false, modifier: Modifier = Modifier) {
    val shape = RoundedCornerShape(size)
    if (hollow) {
        Box(modifier.size(size).border(1.2.dp, color, shape))
    } else {
        Box(modifier.size(size).background(color, shape))
    }
}

@Composable
fun Divider(modifier: Modifier = Modifier) {
    Box(modifier.background(Kampr.tokens.color.line))
}

@Composable
fun PrimaryAction(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    style: TextStyle = Kampr.tokens.type.button,
    vertical: Dp = 15.dp,
    enabled: Boolean = true,
) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.md)
    Box(
        modifier
            .background(if (enabled) tokens.color.accent else tokens.color.raise, shape)
            .edge(tokens.card, shape)
            .let { if (enabled) it.clickable(onClick = onClick) else it }
            .padding(vertical = vertical),
        contentAlignment = Alignment.Center,
    ) {
        KText(text, style, if (enabled) tokens.color.onAccent else tokens.color.mute)
    }
}

@Composable
fun QuietAction(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    style: TextStyle = Kampr.tokens.type.buttonSmall,
    vertical: Dp = 10.dp,
) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.md)
    Box(
        modifier
            .background(tokens.color.raise, shape)
            .edge(tokens.card, shape)
            .clickable(onClick = onClick)
            .padding(vertical = vertical),
        contentAlignment = Alignment.Center,
    ) {
        KText(text, style, tokens.color.text)
    }
}

@Composable
fun Segmented(
    options: List<String>,
    selectedIndex: Int,
    onSelect: (Int) -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val outer = RoundedCornerShape(tokens.radii.md)
    val inner = RoundedCornerShape(tokens.radii.sm)
    Row(
        modifier
            .background(tokens.color.surface, outer)
            .edge(tokens.card, outer)
            .padding(4.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        options.forEachIndexed { index, option ->
            val active = index == selectedIndex
            Box(
                Modifier
                    .weight(1f)
                    .let { if (active) it.background(tokens.color.raise, inner) else it }
                    .clickable { onSelect(index) }
                    .padding(vertical = 8.dp),
                contentAlignment = Alignment.Center,
            ) {
                KText(
                    option,
                    if (active) tokens.type.tab else tokens.type.tab.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.W500),
                    if (active) tokens.color.text else tokens.color.dim,
                )
            }
        }
    }
}

@Composable
fun Gap(width: Dp) {
    Box(Modifier.width(width))
}
