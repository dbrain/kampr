package dev.kampr.shared.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.ServerMsg

// Search this pane's **whole** scrollback, which is not what is on screen and not even what this
// client holds: `pane.read recent` caps at 1000 rows with no offset, so the node asks herdr's
// `pane.copy_search` and answers with positions in rows from the live row (probe #511).
//
// That coordinate is the reason a result list is worth having at all. It is the one indexing
// herdr's history and this client's ring share, so a hit can be aimed at — and a hit deeper than
// the ring carries the row it matched, so the reader still sees the line where they cannot scroll
// to it. A list that silently dropped those would be a count that does not match what it shows.
@Composable
fun FindSheet(
    paneId: String,
    subject: String,
    breakpoint: Breakpoint,
    found: ServerMsg.Found?,
    onSearch: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    val tokens = Kampr.tokens
    var query by remember(paneId) { mutableStateOf("") }
    BottomSheet(breakpoint, onDismiss) {
        SheetHeader("Find in scrollback", subject, null, onDismiss, breakpoint != Breakpoint.Portrait)
        Column(
            Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(start = 20.dp, end = 20.dp, top = 6.dp, bottom = 18.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                KField(
                    hint = "Text to find",
                    text = query,
                    modifier = Modifier.weight(1f),
                    style = tokens.type.body,
                    label = "What to look for in this pane's whole scrollback",
                    onSubmit = { onSearch(query) },
                    onText = { query = it },
                )
                CommandButton("Search", { onSearch(query) }, KamprIcons.search, primary = true, label = "Search this pane's whole scrollback")
            }
            KText("Searches the pane's whole history, not only the rows on screen.", tokens.type.captionSmall, tokens.color.mute, maxLines = 2)
            results(paneId, found)
        }
    }
}

@Composable
private fun ColumnScope.results(paneId: String, found: ServerMsg.Found?) {
    val tokens = Kampr.tokens
    val mine = found?.takeIf { it.pane == paneId } ?: return
    if (mine.matches.isEmpty()) {
        KText("No match in this pane's scrollback.", tokens.type.caption, tokens.color.mute)
        return
    }
    // `total` is every match the node found; the list it hands over is capped so that one search
    // cannot become hundreds of round trips. Saying both is the honest summary.
    val shown = mine.matches.size
    KText(
        if (mine.total > shown) "${mine.total} matches · showing the first $shown" else "${mine.total} matches",
        tokens.type.meta,
        tokens.color.mute,
    )
    for (hit in mine.matches) {
        val line = hit.text.trim()
        // Read, not aimed at. Every row here carries the line it matched, so the search is
        // answered on this screen — but the terminal surface scrolls in pixels over a laid-out
        // grid and this list is in rows from the live row, and a tap that scrolled to roughly the
        // right place would be worse than one that does not pretend to. The TUI, whose scroll is
        // already in rows, does jump.
        Row(
            Modifier.fillMaxWidth().padding(vertical = 5.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            KText("${hit.fromBottom}↑", tokens.type.meta, tokens.color.mute)
            KText(line, tokens.type.caption, tokens.color.text, Modifier.weight(1f))
        }
    }
}
