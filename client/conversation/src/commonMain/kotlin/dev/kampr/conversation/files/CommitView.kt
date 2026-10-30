package dev.kampr.conversation.files

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.conversation.ConversationIcons
import dev.kampr.conversation.DiffCard
import dev.kampr.shared.net.commitAttachmentId
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.FilesNav
import dev.kampr.shared.ui.FilesPlace
import dev.kampr.shared.ui.GlyphTarget
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.KamprIcons
import dev.kampr.shared.ui.LANDSCAPE_TOUCH
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.touchable

// A commit that touches more files than this opens folded, so the list of what it touched is
// what shows first rather than the first file's patch.
private const val OPEN_UP_TO = 6

@Composable
fun CommitView(place: FilesPlace.Commit, nav: FilesNav, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    var attempt by remember(place) { mutableIntStateOf(0) }
    val load = rememberLoad(place.anchor, commitAttachmentId(place.repo, place.sha), attempt) { got -> textOf(got)?.let(::splitCommit) }

    Column(modifier.fillMaxSize().background(tokens.color.bg)) {
        FilesBar(place.sha.take(7), baseName(place.repo), nav)
        LoadGate(load, "this commit", { attempt++ }) { parts -> Parts(place, parts, nav) }
    }
}

@Composable
private fun Parts(place: FilesPlace.Commit, parts: CommitParts, nav: FilesNav) {
    val tokens = Kampr.tokens
    val safe = LocalSafeArea.current
    LazyColumn(
        Modifier.fillMaxSize(),
        contentPadding = PaddingValues(start = 12.dp + safe.left, end = 12.dp + safe.right, top = 10.dp, bottom = 16.dp + safe.bottom),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        item(key = "header") {
            SelectionContainer {
                KText(
                    parts.header,
                    tokens.type.caption.copy(fontFamily = tokens.fonts.mono),
                    tokens.color.dim,
                    Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),
                    maxLines = Int.MAX_VALUE,
                )
            }
        }
        itemsIndexed(parts.files, key = { index, patch -> "$index-${patch.path}" }) { _, patch ->
            FileSection(patch, parts.files.size <= OPEN_UP_TO) {
                nav.open(FilesPlace.File(place.nodeId, place.anchor, childPath(place.repo, patch.path)))
            }
        }
    }
}

@Composable
private fun FileSection(patch: FilePatch, startOpen: Boolean, onOpen: () -> Unit) {
    val tokens = Kampr.tokens
    var open by remember(patch.path) { mutableStateOf(startOpen) }
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            GlyphTarget(
                if (open) ConversationIcons.chevronDown else KamprIcons.chevronRight,
                if (open) "Fold the patch for ${patch.path}" else "Show the patch for ${patch.path}",
                tokens.color.mute,
                { open = !open },
                target = LANDSCAPE_TOUCH,
                glyph = 13.dp,
            )
            KText(
                patch.path,
                tokens.type.meta,
                tokens.color.accent,
                Modifier
                    .weight(1f)
                    .touchable(LANDSCAPE_TOUCH)
                    .action("Open ${patch.path}", onOpen)
                    .padding(vertical = 9.dp),
            )
        }
        if (open) DiffCard(null, patch.text, "", Modifier.fillMaxWidth(), title = patch.path.substringAfterLast('/'))
    }
}
