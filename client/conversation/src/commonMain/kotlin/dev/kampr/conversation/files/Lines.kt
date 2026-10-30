package dev.kampr.conversation.files

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicText
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.max
import dev.kampr.conversation.ConversationPalette
import dev.kampr.conversation.highlight
import dev.kampr.conversation.rememberConversationPalette
import dev.kampr.conversation.sizeWords
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.QuietAction
import dev.kampr.shared.ui.announce
import dev.kampr.shared.ui.glyphFallback

private const val CHUNK_LINES = 100
private const val CHUNKS_KEPT = 48
private const val READ_AHEAD = 200

// One line shaped as one text node is fine until the line is a minified bundle: shaping two
// million characters is a frame nobody gets back. The rest of such a line is still in Copy.
private const val LONGEST_DRAWN = 2_000

private fun drawn(line: String): String {
    val bare = line.removeSuffix("\r")
    return if (bare.length <= LONGEST_DRAWN) bare else bare.take(LONGEST_DRAWN) + " … ${bare.length - LONGEST_DRAWN} more characters"
}

// Highlighting is scanned a hundred lines at a time — enough context for most constructs, and the
// unit a scroll brings into view — and kept, so a line scrolled back to is not scanned again. A
// chunk still filling is scanned again once it holds more lines.
private class Highlights(private val lang: String?, private val palette: ConversationPalette) {
    private val chunks = LinkedHashMap<Int, List<AnnotatedString>>()

    fun line(lines: List<String>, index: Int): AnnotatedString {
        val chunk = index / CHUNK_LINES
        val from = chunk * CHUNK_LINES
        val to = minOf(lines.size, from + CHUNK_LINES)
        val rows = chunks[chunk]?.takeIf { it.size == to - from } ?: scan(lines.subList(from, to)).also {
            chunks.remove(chunk)
            chunks[chunk] = it
            while (chunks.size > CHUNKS_KEPT) chunks.remove(chunks.keys.first())
        }
        return rows[index - from]
    }

    private fun scan(lines: List<String>): List<AnnotatedString> {
        val shown = lines.map(::drawn)
        val marked = highlight(shown.joinToString("\n"), lang, palette)
        var at = 0
        return shown.map { line ->
            marked.subSequence(at, at + line.length).also { at += line.length + 1 }
        }
    }
}

@Composable
fun NumberedLines(stream: TextStream, lang: String?, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val safe = LocalSafeArea.current
    val palette = rememberConversationPalette()
    val style = tokens.type.caption.copy(fontFamily = tokens.fonts.mono, color = palette.plain)
    val highlights = remember(stream, lang, palette) { Highlights(lang, palette) }
    val measurer = rememberTextMeasurer()
    val density = LocalDensity.current
    val glyph = remember(style, density) { with(density) { measurer.measure("0", style).size.width.toDp() } }
    val list = rememberLazyListState()

    val lines = stream.version.let { stream.lines.lines }
    val count = lines.size
    val gutter = glyph * maxOf(3, count.toString().length) + 10.dp
    val body = glyph * minOf(stream.lines.widest, LONGEST_DRAWN + 24) + 12.dp

    LaunchedEffect(stream, list) {
        snapshotFlow { Triple(list.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0, stream.version, stream.failed) }
            .collect { (last, _, _) ->
                if (!stream.complete && stream.failed == null && last >= stream.lines.lines.size - READ_AHEAD) stream.more()
            }
    }

    BoxWithConstraints(modifier.fillMaxWidth()) {
        val wide = max(maxWidth, gutter + body + safe.left + safe.right + 24.dp)
        SelectionContainer(Modifier.horizontalScroll(rememberScrollState())) {
            LazyColumn(
                Modifier.width(wide).fillMaxHeight(),
                state = list,
                contentPadding = PaddingValues(start = 12.dp + safe.left, end = 12.dp + safe.right, top = 4.dp, bottom = 16.dp),
            ) {
                items(count, key = { it }) { index ->
                    Line(index + 1, highlights.line(lines, index), gutter, style)
                }
                item(key = "end") { End(stream) }
            }
        }
    }
}

@Composable
private fun Line(number: Int, text: AnnotatedString, gutter: Dp, style: TextStyle) {
    val tokens = Kampr.tokens
    Row(Modifier.fillMaxWidth()) {
        DisableSelection {
            BasicText(
                number.toString(),
                Modifier.width(gutter).padding(end = 10.dp),
                style = style.copy(color = tokens.color.mute, textAlign = TextAlign.End),
                softWrap = false,
            )
        }
        BasicText(text.glyphFallback(style), style = style, softWrap = false)
    }
}

@Composable
private fun End(stream: TextStream) {
    val tokens = Kampr.tokens
    if (stream.complete) return
    val failed = stream.failed
    val read = sizeWords(stream.lines.bytes)
    val of = stream.total?.let { " of ${sizeWords(it)}" }.orEmpty()
    DisableSelection {
        Column(Modifier.fillMaxWidth().padding(vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            val words = if (failed == null) "read $read$of — reading on" else "read $read$of — $failed"
            KText(words, tokens.type.micro, if (failed == null) tokens.color.mute else tokens.color.blocked, Modifier.announce(words))
            if (failed != null) QuietAction("Read on", { stream.failed = null }, label = "Try reading the rest again")
        }
    }
}
