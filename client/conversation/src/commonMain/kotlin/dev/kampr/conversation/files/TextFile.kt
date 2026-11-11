package dev.kampr.conversation.files

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import dev.kampr.conversation.md.Markdown
import dev.kampr.conversation.syntax.langOf
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.QuietAction
import dev.kampr.shared.ui.Segmented
import dev.kampr.shared.ui.announce
import kotlinx.coroutines.launch

private sealed interface Changes {
    data object Asking : Changes
    data class Got(val patches: List<Patch>) : Changes
}

private fun extension(name: String) = name.substringAfterLast('.', "").lowercase()

private fun isMarkdown(name: String) = extension(name) in setOf("md", "markdown")

private fun isJson(name: String) = extension(name) in setOf("json", "jsonc")

// A first line this long is a document written for a machine; laid out by hand, JSON's first line
// is a bracket. No whole line yet means the first one is longer than the slice read.
private const val MINIFIED_LINE = 200

private fun minified(stream: TextStream) = stream.lines.lines.firstOrNull()?.let { it.length > MINIFIED_LINE } ?: (stream.lines.bytes > 0)

@Composable
fun TextFile(anchor: String, name: String, path: String?, stream: TextStream, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val io = LocalPaneIo.current
    var changes by remember(path) { mutableStateOf<Changes>(Changes.Asking) }
    if (path != null) {
        LaunchedEffect(anchor, path) { changes = Changes.Got(fetchChanges(io, anchor, path)) }
    }
    var tab by rememberSaveable(path) { mutableIntStateOf(0) }
    Column(modifier.fillMaxSize()) {
        if (path != null) {
            val first = (changes as? Changes.Got)?.patches?.firstOrNull()
            val label = first?.let { "Changes +${it.counts.added} −${it.counts.removed}" } ?: "Changes"
            DisableSelection {
                Segmented(
                    listOf("Contents", label),
                    tab,
                    { tab = it },
                    Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp),
                    what = "tab",
                )
            }
        }
        if (tab == 0 || path == null) {
            Contents(name, stream, Modifier.weight(1f))
        } else {
            when (val c = changes) {
                Changes.Asking -> Skeleton(SkeletonShape.Lines, "the changes")

                is Changes.Got -> if (c.patches.isEmpty()) {
                    Note("No changes since HEAD — the file is unchanged, untracked, or not in a git work tree.", tokens.color.mute)
                } else {
                    ChangesView(c.patches, Modifier.weight(1f))
                }
            }
        }
    }
}

@Composable
private fun Note(words: String, tint: Color) {
    KText(words, Kampr.tokens.type.caption, tint, Modifier.fillMaxWidth().padding(16.dp).announce(words), maxLines = 4)
}

// A file with a reading other than its source lines — markdown laid out, a spreadsheet as a table —
// opens on that reading, and the source is a press away.
@Composable
private fun Contents(name: String, stream: TextStream, modifier: Modifier) {
    val markdown = isMarkdown(name)
    val delimiter = remember(name, stream) { csvDelimiter(name, stream.lines.lines.firstOrNull().orEmpty()) }
    val json = isJson(name)
    val reading = when {
        markdown -> "Rendered"
        delimiter != null -> "Table"
        json -> "Formatted"
        else -> null
    }
    var rendered by rememberSaveable(name) { mutableStateOf(if (json) minified(stream) else reading != null) }
    Column(modifier.fillMaxWidth()) {
        // Said because it was judged from the bytes rather than declared by them, and a reader
        // looking at mojibake needs to know which guess produced it.
        stream.lines.encoding.label?.let { label ->
            KText("Read as $label", Kampr.tokens.type.micro, Kampr.tokens.color.mute, Modifier.padding(horizontal = 12.dp).padding(bottom = 6.dp))
        }
        if (reading != null) {
            DisableSelection {
                Segmented(
                    listOf(reading, "Source"),
                    if (rendered) 0 else 1,
                    { rendered = it == 0 },
                    Modifier.fillMaxWidth().padding(horizontal = 12.dp).padding(bottom = 8.dp),
                    what = "view",
                    compact = true,
                )
            }
        }
        when {
            !rendered -> NumberedLines(stream, langOf(name), Modifier.weight(1f))
            delimiter != null -> CsvTable(stream, delimiter, Modifier.weight(1f))
            json -> Formatted(stream, Modifier.weight(1f))
            else -> Rendered(stream, Modifier.weight(1f))
        }
    }
}

// Markdown is laid out whole, so it is rendered from what has been read and the rest is a press
// away rather than read behind the reader's back.
@Composable
private fun Rendered(stream: TextStream, modifier: Modifier) {
    val safe = LocalSafeArea.current
    val text = remember(stream.version) { stream.lines.text() }
    SelectionContainer(modifier) {
        Column(
            Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(start = 12.dp + safe.left, end = 12.dp + safe.right, top = 4.dp, bottom = 16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Markdown(text, "")
            ReadOn(stream)
        }
    }
}

// Formatting needs the whole document, so like markdown it is laid out from what has been read
// and the rest is a press away.
@Composable
private fun Formatted(stream: TextStream, modifier: Modifier) {
    val formatted = remember(stream.version) { TextStream.whole(formatJson(stream.lines.text()).encodeToByteArray(), "application/json") }
    Column(modifier.fillMaxWidth()) {
        NumberedLines(formatted, "json", Modifier.weight(1f))
        Column(Modifier.padding(horizontal = 12.dp)) { ReadOn(stream) }
    }
}

@Composable
private fun ReadOn(stream: TextStream) {
    val tokens = Kampr.tokens
    val scope = rememberCoroutineScope()
    if (stream.complete) return
    DisableSelection {
        QuietAction(
            if (stream.loading) "Reading…" else "Read on",
            {
                stream.failed = null
                scope.launch { stream.more() }
            },
            Modifier.fillMaxWidth(),
            label = "Read more of the file",
        )
    }
    stream.failed?.let { KText(it, tokens.type.micro, tokens.color.blocked) }
}
