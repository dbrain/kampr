package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LabelText
import dev.kampr.shared.ui.LANDSCAPE_TOUCH
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.edgeBottom
import dev.kampr.shared.ui.touchable

// What the reader is standing in the middle of, and the key that puts it away. Only a row that
// has something to put away is worth pinning — a foldable turn, or a run of tool calls — so a bar
// that appears is always a bar that does something.
class PinnedTurn(val row: TranscriptRow, val index: Int, val key: String)

fun pinKeyOf(row: TranscriptRow, query: String): String? = when (row) {
    is TranscriptRow.Run -> row.key
    is TranscriptRow.One -> foldKey(row.turn)?.takeIf { !turnMatches(row.turn, query) }
}

// The row occupying the top of the viewport, and only while its own header is above the fold.
// Read off `layoutInfo` rather than composed as a `stickyHeader`: a sticky header is a list item,
// so every turn would need one, which doubles the item count and moves every index the search
// hits, the paging trigger and the follow-the-end scroll are all counted in.
fun pinnedTurn(state: LazyListState, rows: List<TranscriptRow>, leading: Int, query: String): PinnedTurn? {
    val layout = state.layoutInfo
    val top = layout.viewportStartOffset
    val item = layout.visibleItemsInfo.firstOrNull { it.offset + it.size > top } ?: return null
    if (item.offset >= top) return null
    val at = item.index - leading
    val row = rows.getOrNull(at) ?: return null
    val key = pinKeyOf(row, query) ?: return null
    return PinnedTurn(row, at, key)
}

@Composable
fun PinnedTurnBar(
    pinned: PinnedTurn,
    agent: String?,
    now: Double,
    onCollapse: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    when (val row = pinned.row) {
        is TranscriptRow.One -> {
            val skin = speakerSkin(speakerOf(row.turn), agent)
            val gist = turnGist(row.turn)
            val held = headerLabel(skin, turnStamp(row.turn.at, now), gist, groupBlocks(row.turn.blocks).size)
            PinnedBar(modifier, "Put away the message of $held", onCollapse) {
                LabelText(skin.label, tokens.type.metaSmall, skin.rail)
                KText(gist, tokens.type.meta, tokens.color.dim, Modifier.weight(1f))
            }
        }
        is TranscriptRow.Run -> {
            val names = row.tools.map { it.name }.distinct()
            val held = "${row.tools.size} tool calls, ${names.joinToString(", ")}"
            PinnedBar(modifier, "Put away $held", onCollapse) {
                LabelText("${row.tools.size} calls", tokens.type.metaSmall, tokens.color.dim)
                KText(names.joinToString(" · "), tokens.type.meta, tokens.color.dim, Modifier.weight(1f))
            }
        }
    }
}

@Composable
private fun PinnedBar(
    modifier: Modifier,
    label: String,
    onCollapse: () -> Unit,
    content: @Composable RowScope.() -> Unit,
) {
    val tokens = Kampr.tokens
    DisableSelection {
        Row(
            modifier
                .fillMaxWidth()
                .background(tokens.color.bar)
                .edgeBottom()
                .touchable(LANDSCAPE_TOUCH)
                .action(label, onCollapse)
                .padding(horizontal = 16.dp, vertical = 7.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            content()
            IconGlyph(ConversationIcons.chevronUp, 12.dp, tokens.color.mute)
        }
    }
}
