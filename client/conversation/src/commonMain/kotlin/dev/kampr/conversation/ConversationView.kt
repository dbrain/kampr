package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.platform.LocalReduceMotion
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LabelText
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.QuietAction
import dev.kampr.shared.ui.Surface
import dev.kampr.shared.ui.GlyphTarget
import dev.kampr.shared.ui.LANDSCAPE_TOUCH
import dev.kampr.shared.ui.announce
import dev.kampr.shared.ui.asHeading
import dev.kampr.shared.ui.edge
import dev.kampr.shared.ui.edgeBottom
import dev.kampr.shared.ui.named
import dev.kampr.shared.ui.readingOrder
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PaneInfo

@Composable
fun ConversationView(pane: PaneState, info: PaneInfo?, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val io = LocalPaneIo.current

    if (info != null && !info.hasConversation) {
        AbsentConversation(info, modifier) { io.show(PaneView.Terminal) }
        return
    }

    val turns = pane.turns
    var query by remember { mutableStateOf("") }
    var searching by remember { mutableStateOf(false) }
    var focus by remember { mutableStateOf(0) }
    val expanded = remember { mutableStateListOf<String>() }
    val listState = rememberLazyListState()
    val hits = remember(pane.revision, query) { searchHits(turns, query) }
    val leading = if (pane.convoMore) 1 else 0

    val stillness = LocalReduceMotion.current
    LaunchedEffect(hits, focus) {
        val target = hits.getOrNull(focus) ?: return@LaunchedEffect
        if (stillness) listState.scrollToItem(target + leading)
        else listState.animateScrollToItem(target + leading)
    }

    // Paging backwards is the whole point of the opaque cursor: ask once per cursor, and let the
    // node decide there is nothing older by clearing `more`.
    var asked by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(pane.id) {
        snapshotFlow { listState.firstVisibleItemIndex }.collect { first ->
            val cursor = pane.convoCursor
            if (first <= 1 && pane.convoMore && cursor != null && cursor != asked) {
                asked = cursor
                io.send(ClientMsg.ConvoLoad(pane.id, cursor))
            }
        }
    }

    val atBottom by remember(pane) {
        derivedStateOf {
            val last = listState.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: return@derivedStateOf true
            last >= turns.lastIndex + (if (pane.convoMore) 1 else 0) - 1
        }
    }
    LaunchedEffect(turns.size) {
        if (atBottom && turns.isNotEmpty()) listState.scrollToItem(turns.lastIndex + leading)
    }

    Column(modifier.fillMaxSize().background(tokens.color.bg)) {
        TranscriptBar(
            count = turns.size,
            searching = searching,
            query = query,
            hits = hits.size,
            focus = focus,
            onQuery = { query = it; focus = 0 },
            onSearching = { searching = it; if (!it) query = "" },
            onStep = { step -> if (hits.isNotEmpty()) focus = (focus + step + hits.size) % hits.size },
            agent = info?.agent,
        )

        Box(Modifier.weight(1f).fillMaxWidth()) {
            if (turns.isEmpty()) {
                KText(
                    "waiting for the transcript",
                    tokens.type.caption,
                    tokens.color.mute,
                    Modifier.align(Alignment.Center),
                )
            }
            LazyColumn(
                state = listState,
                modifier = Modifier.fillMaxSize(),
                contentPadding = androidx.compose.foundation.layout.PaddingValues(
                    start = 16.dp, end = 16.dp, top = 12.dp, bottom = 16.dp,
                ),
                verticalArrangement = Arrangement.spacedBy(14.dp),
            ) {
                if (pane.convoMore) {
                    item(key = "older") {
                        Row(
                            Modifier.fillMaxWidth().announce("Loading earlier turns").padding(bottom = 2.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            IconGlyph(ConversationIcons.history, 12.dp, tokens.color.mute)
                            KText("loading earlier turns", tokens.type.meta, tokens.color.mute)
                        }
                    }
                }
                items(turns, key = { it.id }) { turn ->
                    TurnView(
                        turn = turn,
                        query = query,
                        expanded = expanded,
                        onToggle = { key -> if (key in expanded) expanded.remove(key) else expanded.add(key) },
                    )
                }
            }
        }

        if (!io.readOnly) {
            pane.pending?.let { PendingStrip(it, onAnswer = { key -> io.send(ClientMsg.Answer(pane.id, key)) }) }
        }

        Composer(
            agent = info?.agent,
            enabled = !io.readOnly,
            onSend = { text -> replyMessages(pane.id, text).forEach(io::send) },
        )
    }
}

@Composable
private fun TranscriptBar(
    count: Int,
    searching: Boolean,
    query: String,
    hits: Int,
    focus: Int,
    onQuery: (String) -> Unit,
    onSearching: (Boolean) -> Unit,
    onStep: (Int) -> Unit,
    agent: String?,
) {
    val tokens = Kampr.tokens
    Row(
        Modifier
            .fillMaxWidth()
            .background(tokens.color.bar)
            .edgeBottom()
            .readingOrder(-1f)
            .padding(horizontal = 16.dp, vertical = 9.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        if (!searching) {
            LabelText("transcript", tokens.type.metaSmall, tokens.color.mute)
            KText(
                "$count turns",
                tokens.type.meta,
                tokens.color.mute,
                Modifier
                    .weight(1f)
                    .asHeading()
                    .named("Transcript of ${agent ?: "this pane"}, $count turns"),
            )
            GlyphTarget(
                ConversationIcons.search, "Search the transcript", tokens.color.dim,
                { onSearching(true) }, target = LANDSCAPE_TOUCH, glyph = 15.dp,
            )
            return@Row
        }
        SearchField(query, onQuery, Modifier.weight(1f))
        val tally = if (query.length < 2) "type to search" else if (hits == 0) "no matches" else "${focus + 1}/$hits"
        KText(
            tally,
            tokens.type.meta,
            if (hits == 0 && query.length >= 2) tokens.color.blocked else tokens.color.mute,
            Modifier.announce(
                when {
                    query.length < 2 -> "Type to search"
                    hits == 0 -> "No matches"
                    else -> "Match ${focus + 1} of $hits"
                },
            ),
        )
        GlyphTarget(
            ConversationIcons.up, "Previous match", tokens.color.dim,
            { onStep(-1) }, target = LANDSCAPE_TOUCH, glyph = 13.dp,
        )
        GlyphTarget(
            ConversationIcons.down, "Next match", tokens.color.dim,
            { onStep(1) }, target = LANDSCAPE_TOUCH, glyph = 13.dp,
        )
        GlyphTarget(
            ConversationIcons.close, "Close search", tokens.color.mute,
            { onSearching(false) }, target = LANDSCAPE_TOUCH, glyph = 13.dp,
        )
    }
}

@Composable
private fun SearchField(query: String, onQuery: (String) -> Unit, modifier: Modifier) {
    val tokens = Kampr.tokens
    var value by remember { mutableStateOf(TextFieldValue(query)) }
    val pill = RoundedCornerShape(tokens.radii.sm)
    Box(
        modifier
            .background(tokens.color.surface, pill)
            .edge(tokens.card, pill)
            .padding(horizontal = 10.dp, vertical = 7.dp),
    ) {
        if (value.text.isEmpty()) KText("search the transcript", tokens.type.caption, tokens.color.mute)
        BasicTextField(
            value = value,
            onValueChange = { value = it; onQuery(it.text) },
            modifier = Modifier.fillMaxWidth().named("Search the transcript"),
            textStyle = tokens.type.caption.copy(color = tokens.color.text),
            cursorBrush = SolidColor(tokens.color.accent),
        )
    }
}

// A harness with no journal adapter has no conversation. That is a fact about the pane, not a
// failure, so it reads as one and hands the reader the view that does work.
@Composable
private fun AbsentConversation(info: PaneInfo, modifier: Modifier, onTerminal: () -> Unit) {
    val tokens = Kampr.tokens
    Box(modifier.fillMaxSize().background(tokens.color.bg), contentAlignment = Alignment.Center) {
        Surface(Modifier.widthIn(max = 380.dp).padding(24.dp), radius = tokens.radii.lg) {
            Column(
                Modifier.padding(20.dp),
                verticalArrangement = Arrangement.spacedBy(11.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                IconGlyph(ConversationIcons.speech, 20.dp, tokens.color.mute)
                KText(
                    if (info.agent == null) "This pane is a shell" else "No transcript for ${info.agent}",
                    tokens.type.cardTitle,
                    tokens.color.text,
                )
                KText(
                    if (info.agent == null) {
                        "Its history is the terminal's own scrollback."
                    } else {
                        "This node has no journal adapter for that harness, so there is nothing to read here."
                    },
                    tokens.type.caption,
                    tokens.color.dim,
                    maxLines = 3,
                )
                Box(Modifier.height(2.dp))
                QuietAction(
                    "Open the terminal view", onTerminal, Modifier.fillMaxWidth(),
                    label = "Open the terminal view of this pane instead",
                )
            }
        }
    }
}
