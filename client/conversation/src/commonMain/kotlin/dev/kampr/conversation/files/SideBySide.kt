package dev.kampr.conversation.files

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.text.BasicText
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.conversation.ConversationPalette
import dev.kampr.conversation.rememberConversationPalette
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.glyphFallback

data class SideCell(val number: Int, val text: String, val changed: Boolean)

sealed interface SideRow {
    data class Hunk(val header: String) : SideRow
    data class Pair(val left: SideCell?, val right: SideCell?) : SideRow
}

data class DiffCounts(val added: Int, val removed: Int)

private val HUNK = Regex("""^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@""")

fun sideBySide(diff: String): List<SideRow> {
    val rows = mutableListOf<SideRow>()
    val removed = mutableListOf<SideCell>()
    val added = mutableListOf<SideCell>()
    var inHunk = false
    var left = 0
    var right = 0

    fun flush() {
        for (i in 0 until maxOf(removed.size, added.size)) rows += SideRow.Pair(removed.getOrNull(i), added.getOrNull(i))
        removed.clear()
        added.clear()
    }

    for (line in diff.split('\n')) {
        val header = HUNK.find(line)
        if (header != null) {
            flush()
            inHunk = true
            left = header.groupValues[1].toInt()
            right = header.groupValues[2].toInt()
            rows += SideRow.Hunk(line)
            continue
        }
        if (!inHunk) continue
        when {
            line.startsWith("diff ") -> {
                flush()
                inHunk = false
            }

            line.startsWith('-') -> removed += SideCell(left++, line.drop(1), changed = true)

            line.startsWith('+') -> added += SideCell(right++, line.drop(1), changed = true)

            line.startsWith(' ') -> {
                flush()
                rows += SideRow.Pair(SideCell(left++, line.drop(1), false), SideCell(right++, line.drop(1), false))
            }
        }
    }
    flush()
    return rows
}

fun diffCounts(diff: String): DiffCounts {
    val pairs = sideBySide(diff).filterIsInstance<SideRow.Pair>()
    return DiffCounts(
        added = pairs.count { it.right?.changed == true },
        removed = pairs.count { it.left?.changed == true },
    )
}

// Lazy, because a patch git may now hand over at 64 MiB is far more rows than one column can lay
// out at once.
@Composable
fun SideBySideDiff(diff: String, modifier: Modifier = Modifier) {
    val palette = rememberConversationPalette()
    val rows = remember(diff) { sideBySide(diff) }
    val gutter = remember(rows) { gutterFor(rows) }
    LazyColumn(modifier.fillMaxWidth(), contentPadding = PaddingValues(bottom = 16.dp)) {
        items(rows.size) { index ->
            when (val row = rows[index]) {
                is SideRow.Hunk -> Cell(row.header, palette.hunk, Modifier.fillMaxWidth().background(palette.hunkGround))

                is SideRow.Pair -> Row(Modifier.fillMaxWidth().height(IntrinsicSize.Min)) {
                    Half(row.left, gutter, palette, palette.removed, palette.removedGround, Modifier.weight(1f))
                    Box(Modifier.width(1.dp).fillMaxHeight().background(palette.rule))
                    Half(row.right, gutter, palette, palette.added, palette.addedGround, Modifier.weight(1f))
                }
            }
        }
    }
}

private fun gutterFor(rows: List<SideRow>): Dp {
    val widest = rows.maxOfOrNull { row ->
        if (row is SideRow.Pair) maxOf(row.left?.number ?: 0, row.right?.number ?: 0) else 0
    } ?: 0
    return (widest.toString().length * 7 + 6).dp
}

@Composable
private fun Half(
    cell: SideCell?,
    gutter: Dp,
    palette: ConversationPalette,
    ink: Color,
    ground: Color,
    modifier: Modifier,
) {
    val tokens = Kampr.tokens
    val tinted = if (cell?.changed == true) modifier.background(ground) else modifier
    Row(tinted.fillMaxHeight()) {
        val style = tokens.type.micro.copy(fontFamily = tokens.fonts.mono, color = tokens.color.mute, textAlign = TextAlign.End)
        BasicText(
            cell?.number?.toString() ?: "",
            Modifier.width(gutter).padding(end = 4.dp, top = 1.dp),
            style = style,
        )
        if (cell != null) Cell(cell.text, if (cell.changed) ink else palette.plain, Modifier.weight(1f))
    }
}

@Composable
private fun Cell(text: String, ink: Color, modifier: Modifier) {
    val tokens = Kampr.tokens
    val style = tokens.type.micro.copy(fontFamily = tokens.fonts.mono, color = ink)
    BasicText(
        AnnotatedString(text.ifEmpty { " " }).glyphFallback(style),
        modifier.padding(horizontal = 4.dp, vertical = 1.dp),
        style = style,
    )
}
