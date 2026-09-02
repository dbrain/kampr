package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicText
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.runtime.remember
import androidx.compose.ui.text.AnnotatedString
import dev.kampr.shared.theme.BorderSpec
import dev.kampr.shared.theme.GlyphGaps
import dev.kampr.shared.theme.withGlyphFallback
import dev.kampr.shared.theme.Kampr

@Composable
fun KText(
    text: String,
    style: TextStyle,
    color: Color,
    modifier: Modifier = Modifier,
    maxLines: Int = 1,
    overflow: TextOverflow = TextOverflow.Ellipsis,
) {
    val styled = style.copy(color = color)
    val routed = glyphFallback(text, styled)
    if (routed == null) {
        BasicText(text = text, modifier = modifier, style = styled, maxLines = maxLines, overflow = overflow)
    } else {
        BasicText(text = routed, modifier = modifier, style = styled, maxLines = maxLines, overflow = overflow)
    }
}

// The one seam every piece of prose in this app goes through, and the reason it is here rather
// than in the markdown builder: a tool card's summary, a launched-agent headline and a pane title
// are all prose too, and all of them were drawing `✅` as tofu in a browser.
//
// Null means "nothing to re-aim", which is every ASCII string — the caller then draws the plain
// `String` it already had and nothing is allocated. `remember` keys on the text and the family
// because a transcript redraws far more often than it changes.
@Composable
fun glyphFallback(text: String, style: TextStyle): AnnotatedString? {
    val fonts = Kampr.tokens.fonts
    val gaps = when (style.fontFamily) {
        fonts.mono, fonts.terminal -> if (style.fontFamily == fonts.terminal) GlyphGaps.none else fonts.monoGaps
        else -> fonts.uiGaps
    }
    return remember(text, gaps, fonts.terminal) { text.withGlyphFallback(gaps, fonts.terminal) }
}

// The same, for text that is already styled — the markdown builder's output, which carries its own
// code and link spans and must keep them.
@Composable
fun AnnotatedString.glyphFallback(style: TextStyle): AnnotatedString {
    val fonts = Kampr.tokens.fonts
    val gaps = when (style.fontFamily) {
        fonts.mono, fonts.terminal -> if (style.fontFamily == fonts.terminal) GlyphGaps.none else fonts.monoGaps
        else -> fonts.uiGaps
    }
    val source = this
    return remember(source, gaps, fonts.terminal) { source.withGlyphFallback(gaps, fonts.terminal) }
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

// Four silhouettes, not four colours. A 7 dp square, disc, bar and ring stay apart from each
// other with the hue removed, which is what a status indicator has to do to mean anything to a
// third of colour-blind readers.
enum class MarkShape { Square, Circle, Bar, Ring }

@Composable
fun Mark(color: Color, shape: MarkShape, size: Dp = 8.dp, modifier: Modifier = Modifier) {
    val round = RoundedCornerShape(size)
    val box = modifier.size(size)
    when (shape) {
        MarkShape.Square -> Box(box.background(color, RoundedCornerShape(size * 0.16f)))
        MarkShape.Circle -> Box(box.background(color, round))
        MarkShape.Ring -> Box(box.border(size * 0.18f, color, round))
        MarkShape.Bar -> Box(box, contentAlignment = Alignment.Center) {
            Box(Modifier.fillMaxWidth().height(size * 0.38f).background(color, round))
        }
    }
}

@Composable
fun Dot(color: Color, size: Dp = 8.dp, hollow: Boolean = false, modifier: Modifier = Modifier) {
    Mark(color, if (hollow) MarkShape.Ring else MarkShape.Circle, size, modifier)
}

@Composable
fun Divider(modifier: Modifier = Modifier) {
    Box(modifier.background(Kampr.tokens.color.line))
}

// A button's caption is chrome. Dragging across a screen to copy what it says must not splice
// the words on its buttons into the paste.
@Composable
fun PrimaryAction(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    style: TextStyle = Kampr.tokens.type.button,
    vertical: Dp = 15.dp,
    enabled: Boolean = true,
    label: String? = null,
) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.md)
    Box(
        modifier
            .background(if (enabled) tokens.color.accent else tokens.color.raise, shape)
            .edge(tokens.card, shape)
            .touchable()
            .action(label ?: text, onClick, shape, enabled = enabled)
            .padding(vertical = vertical),
        contentAlignment = Alignment.Center,
    ) {
        DisableSelection { KText(text, style, if (enabled) tokens.color.onAccent else tokens.color.mute) }
    }
}

@Composable
fun QuietAction(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    style: TextStyle = Kampr.tokens.type.buttonSmall,
    vertical: Dp = 10.dp,
    enabled: Boolean = true,
    label: String? = null,
) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.md)
    Box(
        modifier
            .background(tokens.color.raise, shape)
            .edge(tokens.card, shape)
            .touchable()
            .action(label ?: text, onClick, shape, enabled = enabled)
            .padding(vertical = vertical),
        contentAlignment = Alignment.Center,
    ) {
        DisableSelection { KText(text, style, if (enabled) tokens.color.text else tokens.color.mute) }
    }
}

@Composable
fun Segmented(
    options: List<String>,
    selectedIndex: Int,
    onSelect: (Int) -> Unit,
    modifier: Modifier = Modifier,
    what: String = "view",
) {
    val tokens = Kampr.tokens
    val outer = RoundedCornerShape(tokens.radii.md)
    val inner = RoundedCornerShape(tokens.radii.sm)
    Row(
        modifier
            .background(tokens.color.surface, outer)
            .edge(tokens.card, outer)
            .group()
            .padding(4.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        options.forEachIndexed { index, option ->
            val active = index == selectedIndex
            Box(
                Modifier
                    .weight(1f)
                    .let { if (active) it.background(tokens.color.raise, inner) else it }
                    .touchable(LANDSCAPE_TOUCH)
                    .action(
                        "$option $what",
                        { onSelect(index) },
                        inner,
                        role = Role.Tab,
                        selected = active,
                    )
                    .padding(vertical = 8.dp),
                contentAlignment = Alignment.Center,
            ) {
                // Every segment reserves the width its label takes *selected*, whichever segment
                // is. The selected one is W700 and the rest are W500, so without this the control
                // measures wider whenever the longer label is the chosen one — "Conversation" set
                // W700 is not the width it is set W500 — and a header that
                // measures it against whatever width is left over rewraps on the selection alone.
                // That is the report this control's own slot was supposed to have closed: the
                // segment slides out from under the thumb that just tapped it. Measured under a
                // bare font set, the portrait header dropped the switch to a second row and grew
                // it from 195 dp to 371 dp on a change of view.
                //
                // Measured and never drawn — `drawWithContent` that does not call `drawContent`
                // costs a text layout and no paint. A `Box` is as wide as its widest child, and
                // this one is the widest this segment can ever be.
                DisableSelection {
                    KText(
                        option,
                        tokens.type.tab,
                        tokens.color.text,
                        Modifier.clearAndSetSemantics {}.drawWithContent {},
                    )
                    KText(
                        option,
                        if (active) tokens.type.tab else tokens.type.tab.copy(fontWeight = FontWeight.W500),
                        if (active) tokens.color.text else tokens.color.dim,
                    )
                }
            }
        }
    }
}

@Composable
fun Gap(width: Dp) {
    Box(Modifier.width(width))
}

// A glyph in a chip, in a target big enough to hit: the painted chip stays small so a header
// does not grow around it, and the box that catches the tap is the one the touch rule sizes.
@Composable
fun GlyphAction(
    icon: Icon,
    label: String,
    tint: Color,
    target: Dp,
    modifier: Modifier = Modifier,
    chip: Dp = 28.dp,
    onClick: () -> Unit,
) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.sm)
    Box(modifier.size(target).action(label, onClick), contentAlignment = Alignment.Center) {
        Box(
            Modifier.size(chip).background(tokens.color.raise, shape).edge(tokens.card, shape),
            contentAlignment = Alignment.Center,
        ) {
            IconGlyph(icon, chip * 0.54f, tint)
        }
    }
}
