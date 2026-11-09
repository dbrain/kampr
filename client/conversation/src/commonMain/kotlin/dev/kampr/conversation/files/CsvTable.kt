package dev.kampr.conversation.files

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.dp
import dev.kampr.conversation.md.Align
import dev.kampr.conversation.md.TableRow
import dev.kampr.conversation.md.columnWidths
import dev.kampr.conversation.rememberConversationPalette
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.LocalSafeArea

// Columns are sized from the header and the rows read first, once: re-measuring every cell of a
// table that grows a slice at a time is the cost that makes a long sheet stutter, and a column
// that grows later wraps inside its cell rather than moving every column right of it.
private const val SIZED_FROM = 200

private val NUMBER = Regex("""[-+]?[\d,]*\.?\d+([eE][-+]?\d+)?%?""")

// Read as a spreadsheet would show it: the header fixed above rows that scroll, every cell its own
// text with nothing in it read as markup, and a column of numbers set against its right edge.
@Composable
fun CsvTable(stream: TextStream, delimiter: Char, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val safe = LocalSafeArea.current
    val palette = rememberConversationPalette()
    val measurer = rememberTextMeasurer()
    val density = LocalDensity.current
    val list = rememberLazyListState()
    val headStyle = remember(tokens) { tokens.type.metaSmall.copy(color = tokens.color.dim, fontFamily = tokens.fonts.ui) }
    val cellStyle = remember(tokens) { tokens.type.caption.copy(color = tokens.color.text) }

    val records = remember(stream, delimiter) { CsvRecords(delimiter) }
    val rows = remember(stream.version) {
        records.feed(stream.lines.lines, stream.complete)
        records.rows
    }
    val sized = rows.size >= SIZED_FROM + 1 || stream.complete
    val sample = remember(sized, rows.isNotEmpty()) { rows.take(SIZED_FROM + 1) }
    val columns = sample.maxOfOrNull { it.size } ?: 0
    val header = remember(sample) { padded(sample.firstOrNull().orEmpty(), columns).map(::AnnotatedString) }
    val widths = remember(sample, headStyle, cellStyle, density) {
        columnWidths(header, sample.drop(1).map { row -> padded(row, columns).map(::AnnotatedString) }, headStyle, cellStyle, measurer, density)
    }
    val aligns = remember(sample) {
        (0 until columns).map { column ->
            val cells = sample.drop(1).mapNotNull { it.getOrNull(column)?.trim()?.takeIf(String::isNotEmpty) }
            if (cells.isNotEmpty() && cells.all { NUMBER.matches(it) }) Align.End else Align.Start
        }
    }
    val body = (rows.size - 1).coerceAtLeast(0)

    LaunchedEffect(stream, list) {
        snapshotFlow { Triple(list.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0, stream.version, stream.failed) }
            .collect { (last, _, _) ->
                if (!stream.complete && stream.failed == null && last >= records.rows.size - READ_AHEAD) stream.more()
            }
    }

    Column(
        modifier
            .horizontalScroll(rememberScrollState())
            .padding(start = 12.dp + safe.left, end = 12.dp + safe.right),
    ) {
        val wide = Modifier.width(with(density) { widths.sum().toDp() })
        TableRow(header, widths, aligns, headStyle, palette, palette.headerGround, true)
        LazyColumn(wide.weight(1f).fillMaxHeight(), state = list, contentPadding = PaddingValues(bottom = 16.dp)) {
            items(body, key = { it }) { index ->
                TableRow(padded(rows[index + 1], columns).map(::AnnotatedString), widths, aligns, cellStyle, palette, null, true)
            }
            item(key = "end") { End(stream) }
        }
    }
}

private fun padded(row: List<String>, columns: Int): List<String> = if (row.size >= columns) row else row + List(columns - row.size) { "" }
