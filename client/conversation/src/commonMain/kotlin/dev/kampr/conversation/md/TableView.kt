package dev.kampr.conversation.md

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicText
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Density
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.TextMeasurer
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.conversation.ConversationPalette
import dev.kampr.conversation.markMatches
import dev.kampr.conversation.rememberConversationPalette
import dev.kampr.conversation.rememberInlineStyles
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.edge

private val CELL_PAD_X = 10.dp
private val CELL_PAD_Y = 9.dp

// A column wider than this is not readable on a phone anyway, so it wraps inside its own cell
// and the table scrolls for the rest. Without a cap one prose column pushes every other column
// off the far side of a very wide scroll.
private val MAX_COLUMN = 230.dp
private val MIN_COLUMN = 44.dp

@Composable
fun MarkdownTable(table: MdBlock.Table, query: String, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val palette = rememberConversationPalette()
    val inline = rememberInlineStyles(palette)
    val measurer = rememberTextMeasurer()
    val density = LocalDensity.current

    val headStyle = remember(tokens) {
        tokens.type.metaSmall.copy(color = tokens.color.dim, fontFamily = tokens.fonts.ui)
    }
    val cellStyle = remember(tokens) { tokens.type.caption.copy(color = tokens.color.text) }

    val head = remember(table, inline, query, palette) {
        table.header.map { inlineMarkdown(it, inline).markMatches(query, palette.match) }
    }
    val body = remember(table, inline, query, palette) {
        table.rows.map { row -> row.map { inlineMarkdown(it, inline).markMatches(query, palette.match) } }
    }

    val shape = RoundedCornerShape(tokens.radii.md)
    // Sized to its columns, never to the pane. A two-column table stretched across a desktop is a
    // table with a hole in the middle of it, which is what growing the columns to fill produced —
    // and it is not what any terminal, Claude Code's own included, does with one.
    //
    // `BoxWithConstraints` carries no width of its own here: `maxWidth` is the room the frame is
    // offering, and what the box measures to is what the columns came to inside it.
    BoxWithConstraints(modifier) {
        val available = with(density) { maxWidth.toPx() }
        val widths = remember(head, body, headStyle, cellStyle, available, density) {
            columnWidths(head, body, headStyle, cellStyle, measurer, density)
        }
        val scroll = rememberScrollState()
        Column(Modifier.width(with(density) { minOf(widths.sum(), available).toDp() })) {
            Box(Modifier.fillMaxWidth().clip(shape).edge(tokens.chrome, shape)) {
                Column(Modifier.horizontalScroll(scroll)) {
                    TableRow(head, widths, table.aligns, headStyle, palette, palette.headerGround, true)
                    body.forEachIndexed { index, row ->
                        TableRow(row, widths, table.aligns, cellStyle, palette, null, index < body.lastIndex)
                    }
                }
            }
            // The overflow is invisible until you try to scroll, so the table says there is more.
            if (widths.sum() > available) {
                BasicText(
                    text = "swipe the table →",
                    modifier = Modifier.padding(top = 5.dp),
                    style = tokens.type.micro.copy(color = tokens.color.mute),
                )
            }
        }
    }
}

@Composable
private fun TableRow(
    cells: List<AnnotatedString>,
    widths: List<Float>,
    aligns: List<Align>,
    style: TextStyle,
    palette: ConversationPalette,
    ground: Color?,
    ruled: Boolean,
) {
    val density = LocalDensity.current
    Row(
        Modifier
            .let { if (ground != null) it.background(ground) else it }
            .let { if (ruled) it.ruleBottom(palette.rule) else it },
    ) {
        cells.forEachIndexed { index, cell ->
            BasicText(
                text = cell,
                modifier = Modifier
                    .width(with(density) { widths.getOrElse(index) { 0f }.toDp() })
                    .padding(horizontal = CELL_PAD_X, vertical = CELL_PAD_Y),
                style = style.copy(
                    textAlign = when (aligns.getOrElse(index) { Align.Start }) {
                        Align.Start -> TextAlign.Start
                        Align.Center -> TextAlign.Center
                        Align.End -> TextAlign.End
                    },
                ),
            )
        }
    }
}

private fun Modifier.ruleBottom(color: Color): Modifier = drawBehind {
    drawRect(color, Offset(0f, size.height - 1f), Size(size.width, 1f))
}

// Natural width per column, capped. Never scaled *down*: shrinking to fit is what turns a table
// back into mush, so the overflow goes to the table's own scroller and the page stays put. Never
// scaled *up* either — a column is as wide as the widest thing in it, and a table grown to fill
// the pane is the defect this pairs with.
private fun columnWidths(
    head: List<AnnotatedString>,
    body: List<List<AnnotatedString>>,
    headStyle: TextStyle,
    cellStyle: TextStyle,
    measurer: TextMeasurer,
    density: Density,
): List<Float> {
    fun px(dp: Dp) = with(density) { dp.toPx() }
    val pad = px(CELL_PAD_X) * 2
    val cap = px(MAX_COLUMN)
    val floor = px(MIN_COLUMN)

    return head.indices.map { column ->
        val header = measurer.measure(head[column], headStyle, softWrap = false, maxLines = 1).size.width.toFloat()
        val widest = body.fold(header) { acc, row ->
            val cell = row.getOrNull(column) ?: return@fold acc
            maxOf(acc, measurer.measure(cell, cellStyle, softWrap = false, maxLines = 1).size.width.toFloat())
        }
        (widest + pad).coerceIn(floor, cap)
    }
}
