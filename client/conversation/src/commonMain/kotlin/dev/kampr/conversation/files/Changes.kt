package dev.kampr.conversation.files

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.net.diffAttachmentId
import dev.kampr.shared.net.stagedAttachmentId
import dev.kampr.shared.net.unstagedAttachmentId
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.Segmented
import dev.kampr.shared.ui.announce
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope

enum class Side(val label: String) { All("All"), Staged("Staged"), Unstaged("Unstaged") }

class Patch(val side: Side, val text: String) {
    val counts: DiffCounts by lazy { diffCounts(text) }
}

// A switch only where there is a choice: with changes both in the index and beside it, all three
// differ. With only one kind, "all" is that kind, so it is shown once under its own name — and a
// node too old to answer `staged`/`unstaged` leaves the one diff it always had.
fun changeViews(all: String?, staged: String?, unstaged: String?): List<Patch> {
    val a = all?.takeIf { it.isNotBlank() }
    val s = staged?.takeIf { it.isNotBlank() }
    val u = unstaged?.takeIf { it.isNotBlank() }
    return when {
        s != null && u != null -> listOfNotNull(a?.let { Patch(Side.All, it) }, Patch(Side.Staged, s), Patch(Side.Unstaged, u))
        s != null -> listOf(Patch(Side.Staged, s))
        u != null -> listOf(Patch(Side.Unstaged, u))
        a != null -> listOf(Patch(Side.All, a))
        else -> emptyList()
    }
}

suspend fun fetchChanges(io: PaneIo, anchor: String, path: String): List<Patch> = coroutineScope {
    suspend fun text(id: String) = (io.attachment(anchor, id) as? AttachmentBytes.Ok)?.bytes?.decodeToString()
    val all = async { text(diffAttachmentId(path)) }
    val staged = async { text(stagedAttachmentId(path)) }
    val unstaged = async { text(unstagedAttachmentId(path)) }
    changeViews(all.await(), staged.await(), unstaged.await())
}

@Composable
fun ChangesView(patches: List<Patch>, modifier: Modifier) {
    val tokens = Kampr.tokens
    val safe = LocalSafeArea.current
    var side by remember(patches) { mutableIntStateOf(0) }
    var split by remember { mutableStateOf(false) }
    val patch = patches[side.coerceAtMost(patches.lastIndex)]
    Column(modifier.fillMaxWidth()) {
        DisableSelection {
            if (patches.size > 1) {
                Segmented(
                    patches.map { "${it.side.label} +${it.counts.added} −${it.counts.removed}" },
                    side,
                    { side = it },
                    Modifier.fillMaxWidth().padding(horizontal = 12.dp).padding(bottom = 8.dp),
                    what = "changes",
                    compact = true,
                )
            } else {
                val said = when (patch.side) {
                    Side.Staged -> "All of it is staged."
                    Side.Unstaged -> "None of it is staged."
                    Side.All -> null
                }
                said?.let { KText(it, tokens.type.micro, tokens.color.mute, Modifier.padding(horizontal = 12.dp).padding(bottom = 8.dp).announce(it)) }
            }
            Segmented(
                listOf("Unified", "Side by side"),
                if (split) 1 else 0,
                { split = it == 1 },
                Modifier.fillMaxWidth().padding(horizontal = 12.dp).padding(bottom = 8.dp),
                what = "diff",
                compact = true,
            )
        }
        SelectionContainer(Modifier.weight(1f)) {
            if (split) {
                SideBySideDiff(patch.text, Modifier.fillMaxSize().padding(start = 8.dp + safe.left, end = 8.dp + safe.right))
            } else {
                UnifiedDiff(patch.text, Modifier.fillMaxSize().padding(start = 8.dp + safe.left, end = 8.dp + safe.right))
            }
        }
    }
}
