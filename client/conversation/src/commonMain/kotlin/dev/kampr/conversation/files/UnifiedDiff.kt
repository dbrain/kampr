package dev.kampr.conversation.files

import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.conversation.DiffRow
import dev.kampr.conversation.parseDiff
import dev.kampr.conversation.rememberConversationPalette

// Lines wrap rather than share one horizontal scroller: a lazy list composes only the rows on
// screen, and a scroll position cannot be held in common by rows that come and go.
@Composable
fun UnifiedDiff(diff: String, modifier: Modifier = Modifier) {
    val palette = rememberConversationPalette()
    val lines = remember(diff) { parseDiff(diff).dropLastWhile { it.text.isEmpty() } }
    LazyColumn(modifier.fillMaxWidth(), contentPadding = PaddingValues(bottom = 16.dp)) {
        items(lines.size) { index -> DiffRow(lines[index], "", palette, wrap = true) }
    }
}
