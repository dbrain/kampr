package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.runtime.Composable
import androidx.compose.runtime.ProvidableCompositionLocal
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.SubConversation
import dev.kampr.shared.net.wallClockMillis
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.BackAction
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.KamprIcons
import dev.kampr.shared.ui.LANDSCAPE_TOUCH
import dev.kampr.shared.ui.LabelText
import dev.kampr.shared.ui.QuietAction
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.announce
import dev.kampr.shared.ui.asHeading
import dev.kampr.shared.ui.edgeBottom
import dev.kampr.shared.ui.named
import dev.kampr.shared.ui.readingOrder
import dev.kampr.shared.ui.touchable
import dev.kampr.shared.wire.Block

// What opening a launched conversation does, provided by whatever is holding the transcript.
// Null where nothing can open one — the fallback surface, an artboard — and a card that cannot be
// opened is not drawn at all, because an affordance that does nothing is worse than none.
val LocalOpenSub: ProvidableCompositionLocal<((Block.Sub) -> Unit)?> = staticCompositionLocalOf { null }

// The card's own line: the agent's type and what it was asked to do, which the node reads off the
// harness's own metadata. Both are optional on the wire, so the fallback has to be a sentence
// rather than an empty string.
fun subHeadline(sub: Block.Sub): String {
    val kind = sub.kind?.trim()?.takeIf { it.isNotEmpty() }
    val title = sub.title?.trim()?.takeIf { it.isNotEmpty() }
    return listOfNotNull(kind, title).joinToString(" — ").ifEmpty { "a conversation this turn launched" }
}

// The affordance the operator asked for: *see what the agent is doing by selecting it*. It rides
// beside the tool card that launched it rather than replacing it, because the call and what the
// call started are two different facts about the turn.
@Composable
fun SubCard(sub: Block.Sub, modifier: Modifier = Modifier) {
    val open = LocalOpenSub.current ?: return
    val tokens = Kampr.tokens
    val headline = subHeadline(sub)
    DisableSelection {
        Row(
            modifier
                .fillMaxWidth()
                .touchable(LANDSCAPE_TOUCH)
                .action("Open the conversation with $headline", { open(sub) })
                .padding(horizontal = 12.dp, vertical = 9.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(9.dp),
        ) {
            IconGlyph(ConversationIcons.branch, 14.dp, tokens.color.dim)
            KText(headline, tokens.type.meta, tokens.color.text, Modifier.weight(1f), maxLines = 2)
            IconGlyph(KamprIcons.chevronRight, 12.dp, tokens.color.mute)
        }
    }
}

// A launched conversation, read as one. Not a panel inside the pane's transcript: its turns belong
// to another agent and the wire refuses to inline them for exactly that reason, so this covers the
// transcript and has a way back rather than sitting inside it.
@Composable
fun SubConversationView(
    sub: Block.Sub,
    state: SubConversation?,
    agent: String?,
    now: Double,
    onBack: () -> Unit,
    onOlder: (String) -> Unit,
    modifier: Modifier = Modifier,
    clock: () -> Double = ::wallClockMillis,
) {
    val tokens = Kampr.tokens
    val headline = subHeadline(sub)
    val turns = state?.turns?.filter { it.isVisible() }.orEmpty()
    val expanded = remember(sub.id) { mutableStateListOf<String>() }
    val toggle: (String) -> Unit = { key -> if (key in expanded) expanded.remove(key) else expanded.add(key) }
    val attachments = rememberAttachmentStore("${sub.id}:launched")
    val listState = rememberLazyListState()
    val rows by remember(turns, expanded) { derivedStateOf { transcriptRows(turns, "", expanded) } }
    val stamps = remember(rows, now) { stepStamps(rows, now) }

    Column(modifier.fillMaxSize().background(tokens.color.bg)) {
        DisableSelection {
            Row(
                Modifier
                    .fillMaxWidth()
                    .background(tokens.color.bar)
                    .edgeBottom()
                    .readingOrder(-1f)
                    .padding(end = 16.dp, top = 3.dp, bottom = 3.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                BackAction("Back to the pane's own transcript", onBack, target = LANDSCAPE_TOUCH)
                Column(Modifier.weight(1f)) {
                    LabelText("launched", tokens.type.metaSmall, tokens.color.mute)
                    KText(
                        headline,
                        tokens.type.meta,
                        tokens.color.text,
                        Modifier.asHeading().named("Launched conversation, $headline"),
                        maxLines = 2,
                    )
                }
            }
        }

        SelectionContainer(Modifier.weight(1f)) {
            Box(Modifier.fillMaxSize()) {
                if (turns.isEmpty()) {
                    KText(
                        if (state?.loaded == true) "nothing written down yet" else "fetching the conversation",
                        tokens.type.caption,
                        tokens.color.mute,
                        Modifier.align(Alignment.Center).announce(
                            if (state?.loaded == true) "Nothing written down yet" else "Fetching the conversation",
                        ),
                    )
                }
                LazyColumn(
                    state = listState,
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(start = 16.dp, end = 16.dp, top = 12.dp, bottom = 16.dp),
                    verticalArrangement = Arrangement.spacedBy(0.dp),
                ) {
                    val cursor = state?.cursor
                    if (state?.more == true && cursor != null) {
                        item(key = "older") {
                            DisableSelection {
                                QuietAction(
                                    "Earlier turns",
                                    { onOlder(cursor) },
                                    Modifier.fillMaxWidth().padding(bottom = 8.dp),
                                    label = "Load the earlier turns of $headline",
                                )
                            }
                        }
                    }
                    itemsIndexed(rows, key = { _, row -> row.key }) { at, row ->
                        TranscriptRowView(
                            rows, at, stamps.getOrNull(at), "", expanded, toggle,
                            attachments = attachments, now = now, agent = agent, clock = clock,
                        )
                    }
                }
            }
        }
    }
}
