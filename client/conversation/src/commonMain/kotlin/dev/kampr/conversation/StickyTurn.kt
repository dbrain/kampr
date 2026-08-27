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

// The block the reader is standing in the middle of, and the row that heads it.
// A value, so the derived state it is read through settles: it is recomputed on every scrolled
// pixel and only a `Pinned` that differs from the last one is worth a recomposition.
data class Pinned(val head: TranscriptRow, val index: Int)

// Read off `layoutInfo` rather than composed as a `stickyHeader`: a sticky header is a list item,
// so every block would need one, which moves every index the search hits, the paging trigger and
// the follow-the-end scroll are all counted in — and a reply's steps are separate items precisely
// so a long one is not composed whole.
//
// The head of the block, not the row itself: what the reader wants named while they are eleven
// tool calls deep is the reply those calls belong to, and what they want to put away is all of it.
fun pinnedBlock(state: LazyListState, rows: List<TranscriptRow>, leading: Int): Pinned? {
    val layout = state.layoutInfo
    val top = layout.viewportStartOffset
    val item = layout.visibleItemsInfo.firstOrNull { it.offset + it.size > top } ?: return null
    val at = item.index - leading
    val row = rows.getOrNull(at) ?: return null
    val head = when (row) {
        is TranscriptRow.Head, is TranscriptRow.Ask -> at
        else -> rows.subList(0, at).indexOfLast { it.key == row.block }.takeIf { it >= 0 } ?: return null
    }
    // Nothing to pin while the block's own header is still on the screen under its own power.
    if (head == at && item.offset >= top) return null
    return Pinned(rows[head], head)
}

// A reply is put away by its own key; an ask by its fold, which a short one does not have. What
// cannot be collapsed is still worth naming, so this decides the action and not the bar.
fun collapseKey(head: TranscriptRow): String? = when (head) {
    is TranscriptRow.Head -> head.key
    is TranscriptRow.Ask -> foldKey(head.turn)
    else -> null
}

// Worded apart from the head it stands in for, because both are on the screen at once and a reader
// hearing two identical buttons cannot tell which one they are on.
@Composable
fun PinnedBlockBar(
    pinned: Pinned,
    agent: String?,
    now: Double,
    onCollapse: (() -> Unit)?,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    when (val row = pinned.head) {
        is TranscriptRow.Head -> {
            val stamp = replySpan(row.reply.at, row.reply.until, now)
            val skin = speakerSkin(Speaker.Agent, agent)
            PinnedBar(modifier, onCollapse, "Put away the reply you are inside, ${replyLabel(agent, stamp, row.reply, false)}") {
                LabelText(skin.label, tokens.type.metaSmall, skin.rail)
                KText(replyGist(row.reply), tokens.type.meta, tokens.color.dim, Modifier.weight(1f))
                KText(replyTally(row.reply), tokens.type.micro, tokens.color.mute)
            }
        }
        is TranscriptRow.Ask -> {
            val skin = speakerSkin(Speaker.You, agent)
            val gist = turnGist(row.turn)
            val held = headerLabel(skin, turnStamp(row.turn.at, now), gist, groupBlocks(row.turn.blocks).size)
            PinnedBar(modifier, onCollapse, "Put away the message you are inside, $held") {
                LabelText(skin.label, tokens.type.metaSmall, skin.rail)
                KText(gist, tokens.type.meta, tokens.color.dim, Modifier.weight(1f))
            }
        }
        else -> Unit
    }
}

// A bar with nothing to collapse is still worth drawing — the reader asked to be told what they
// are looking at as much as to be able to put it away — so the action is what is optional here.
@Composable
private fun PinnedBar(
    modifier: Modifier,
    onCollapse: (() -> Unit)?,
    label: String,
    content: @Composable RowScope.() -> Unit,
) {
    val tokens = Kampr.tokens
    DisableSelection {
        Row(
            modifier
                .fillMaxWidth()
                .background(tokens.color.bar)
                .edgeBottom()
                .then(
                    if (onCollapse == null) Modifier
                    else Modifier.touchable(LANDSCAPE_TOUCH).action(label, onCollapse),
                )
                .padding(horizontal = 16.dp, vertical = 7.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            content()
            if (onCollapse != null) IconGlyph(ConversationIcons.chevronUp, 12.dp, tokens.color.mute)
        }
    }
}
