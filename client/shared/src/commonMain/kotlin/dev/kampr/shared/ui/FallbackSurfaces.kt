package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.TextMeasurer
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.surfaceGeometry
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.theme.TerminalPalette
import dev.kampr.shared.theme.terminalPalette
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.PaneInfo

private const val BASE_CELL_SP = 13f

object FallbackSurfaces : PaneSurfaces {
    @Composable
    override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) {
        val tokens = Kampr.tokens
        val palette = remember(tokens) { tokens.terminalPalette() }
        val measurer = rememberTextMeasurer(cacheSize = 0)
        val layouts = remember(tokens) { LayoutCache() }
        val base = remember(tokens) {
            TextStyle(fontFamily = tokens.fonts.terminal, fontSize = BASE_CELL_SP.sp, color = tokens.color.text)
        }
        // A cached grid is dimmed with the theme's own ground rather than a hardcoded wash.
        val scrim = tokens.color.bg.copy(alpha = 0.45f)
        val probe = remember(base) { measurer.measure("M".repeat(32), base) }
        val cellWidth = probe.size.width / 32f
        val cellHeight = probe.size.height.toFloat()

        Box(
            modifier
                .background(tokens.color.surface2)
                .drawBehind {
                    pane.revision.let { drawGrid(pane, measurer, layouts, base, palette, scrim, cellWidth, cellHeight) }
                }
        )
    }

    @Composable
    override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) {
        val tokens = Kampr.tokens
        Column(
            modifier
                .background(tokens.color.bg)
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 12.dp),
            verticalArrangement = Arrangement.spacedBy(13.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                LabelText("transcript", tokens.type.metaSmall, tokens.color.mute)
                Divider(Modifier.weight(1f).height(1.dp))
                KText("${pane.turns.size} turns", tokens.type.meta, tokens.color.mute)
            }
            for (turn in pane.turns) {
                if (turn.role == "user") {
                    Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.CenterEnd) {
                        Surface(
                            Modifier.widthIn(max = 460.dp),
                            background = tokens.color.raise,
                            radius = tokens.radii.md,
                        ) {
                            Column(Modifier.padding(horizontal = 13.dp, vertical = 9.dp)) {
                                for (block in turn.blocks) BlockView(block)
                            }
                        }
                    }
                } else {
                    Column(verticalArrangement = Arrangement.spacedBy(11.dp)) {
                        for (block in turn.blocks) BlockView(block)
                    }
                }
            }
        }
    }

    @Composable
    override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = Unit
}

@Composable
private fun BlockView(block: Block) {
    val tokens = Kampr.tokens
    when (block) {
        is Block.Md -> KText(block.text, tokens.type.body, tokens.color.text, maxLines = 40)
        is Block.Code -> Surface(
            Modifier.fillMaxWidth(),
            background = tokens.color.surface2,
            radius = tokens.radii.md,
        ) {
            Column {
                Row(
                    Modifier.fillMaxWidth().padding(horizontal = 11.dp, vertical = 6.dp),
                    horizontalArrangement = Arrangement.SpaceBetween,
                ) {
                    KText(block.lang ?: "text", tokens.type.meta, tokens.color.mute)
                    KText("Copy", tokens.type.micro, tokens.color.dim)
                }
                Divider(Modifier.fillMaxWidth().height(1.dp))
                KText(
                    block.text,
                    tokens.type.meta.copy(fontFamily = tokens.fonts.mono),
                    tokens.color.text,
                    Modifier.padding(horizontal = 11.dp, vertical = 9.dp),
                    maxLines = 24,
                )
            }
        }
        is Block.Tool -> Surface(
            Modifier.fillMaxWidth(),
            radius = tokens.radii.md,
        ) {
            Row(
                Modifier.padding(horizontal = 12.dp, vertical = 9.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(9.dp),
            ) {
                IconGlyph(KamprIcons.tool, 14.dp, tokens.color.dim)
                KText(
                    listOfNotNull(block.name, block.summary).joinToString(" · "),
                    tokens.type.meta,
                    tokens.color.dim,
                    Modifier.weight(1f),
                )
                block.lines?.let { KText("$it lines", tokens.type.micro, tokens.color.mute) }
                IconGlyph(KamprIcons.chevronRight, 12.dp, tokens.color.mute)
            }
        }
        is Block.Diff -> Surface(
            Modifier.fillMaxWidth(),
            background = tokens.color.surface2,
            radius = tokens.radii.md,
        ) {
            KText(
                block.text,
                tokens.type.meta.copy(fontFamily = tokens.fonts.mono),
                tokens.color.done,
                Modifier.padding(11.dp),
                maxLines = 24,
            )
        }
        is Block.Unknown -> Unit
    }
}

// The surface is scrollback + live grid as one continuous run of rows, with the live
// viewport pinned to the bottom. Zoom is max(fit-width, fit-height), never min, so the
// surface always fills at least one axis and never letterboxes.
private fun DrawScope.drawGrid(
    pane: PaneState,
    measurer: TextMeasurer,
    layouts: LayoutCache,
    base: TextStyle,
    palette: TerminalPalette,
    scrim: Color,
    cellWidth: Float,
    cellHeight: Float,
) {
    val cells = pane.cells
    val cols = cells.cols
    val liveRows = cells.rows
    if (cols == 0 || liveRows == 0) return

    val scrollback = pane.scrollback
    val historyRows = scrollback.historyRows
    val geometry = surfaceGeometry(
        size.width, size.height, cols, liveRows, historyRows, cellWidth, cellHeight,
    )
    val zoom = geometry.zoom
    val rowHeight = cellHeight * zoom
    val colWidth = cellWidth * zoom
    val originY = geometry.originY
    layouts.retune(zoom)

    val defaultBg = palette.background(pane.styles[0])
    val builder = StringBuilder()

    fun paint(col: Int, text: String, styleId: Int, y: Float) {
        val style = pane.styles[styleId]
        val background = palette.background(style)
        if (background.alpha > 0f && background != defaultBg) {
            drawRect(background, Offset(col * colWidth, y), Size(text.length * colWidth, rowHeight))
        }
        if (text.isBlank()) return
        val key = (styleId.toLong() shl 40) xor text.hashCode().toLong() xor (text.length.toLong() shl 20)
        val layout = layouts.get(key) {
            measurer.measure(
                text,
                base.copy(
                    fontSize = (BASE_CELL_SP * zoom).sp,
                    fontWeight = if (style.bold) FontWeight.W700 else FontWeight.W400,
                    fontStyle = if (style.italic) FontStyle.Italic else FontStyle.Normal,
                    textDecoration = decoration(style.underline, style.strike),
                ),
            )
        }
        drawText(layout, color = palette.foreground(style), topLeft = Offset(col * colWidth, y))
    }

    val firstVisible = ((-originY) / rowHeight).toInt().coerceAtLeast(0)
    val lastVisible = (((size.height - originY) / rowHeight).toInt() + 1).coerceAtMost(historyRows + liveRows)

    for (index in firstVisible until lastVisible) {
        val y = originY + index * rowHeight
        if (index < historyRows) {
            val diff = scrollback.row(scrollback.fromTop + index) ?: continue
            var col = 0
            for (run in diff.runs) {
                if (col >= cols) break
                val text = if (col + run.x.length > cols) run.x.take(cols - col) else run.x
                paint(col, text, run.s, y)
                col += text.length
            }
        } else {
            val row = index - historyRows
            var col = 0
            while (col < cols) {
                val styleId = cells.styleAt(col, row)
                var end = col
                while (end < cols && cells.styleAt(end, row) == styleId) end++
                builder.setLength(0)
                for (i in col until end) builder.append(cells.charAt(i, row))
                paint(col, builder.toString(), styleId, y)
                col = end
            }
        }
    }

    if (pane.cursor.visible) {
        drawRect(
            color = palette.foreground(pane.styles[0]).copy(alpha = 0.75f),
            topLeft = Offset(
                pane.cursor.col * colWidth,
                originY + (historyRows + pane.cursor.row) * rowHeight,
            ),
            size = Size(colWidth, rowHeight),
        )
    }

    if (pane.stale) drawRect(color = scrim, size = size)
}

private fun decoration(underline: Boolean, strike: Boolean): TextDecoration? = when {
    underline && strike -> TextDecoration.combine(listOf(TextDecoration.Underline, TextDecoration.LineThrough))
    underline -> TextDecoration.Underline
    strike -> TextDecoration.LineThrough
    else -> null
}

private class LayoutCache {
    private val entries = HashMap<Long, TextLayoutResult>()
    private var zoom = Float.NaN

    fun retune(value: Float) {
        if (value != zoom) {
            entries.clear()
            zoom = value
        }
    }

    fun get(key: Long, build: () -> TextLayoutResult): TextLayoutResult =
        entries.getOrPut(key, build)
}
