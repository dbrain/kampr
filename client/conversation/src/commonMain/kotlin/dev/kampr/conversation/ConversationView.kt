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
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.net.wallClockMillis
import dev.kampr.shared.platform.LocalReduceMotion
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LabelText
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.LocalSafeArea
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
import kotlinx.coroutines.launch

// Compose can aim a lazy list at an item's *top* and at nothing else, so a transcript that ends
// on a long answer opened at the start of that answer — which read as "somewhere above the
// bottom", by however tall the newest message happened to be, or as the top outright when the
// newest message was the whole screen. An offset past the end of any item is measured against the
// end of the list, which is the only thing that means "the bottom" here.
private const val END_OF_THE_ITEM = Int.MAX_VALUE

@Composable
fun ConversationView(pane: PaneState, info: PaneInfo?, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val io = LocalPaneIo.current

    if (info != null && !info.hasConversation) {
        AbsentConversation(info, modifier) { io.show(PaneView.Terminal) }
        return
    }

    // A live preview is withdrawn by arriving again with no blocks, so an empty turn is a turn
    // that is no longer there.
    val turns = remember(pane.revision) { pane.turns.filter { it.isVisible() } }
    var query by remember { mutableStateOf("") }
    var searching by remember { mutableStateOf(false) }
    var focus by remember { mutableStateOf(0) }
    val expanded = remember { mutableStateListOf<String>() }
    val toggle: (String) -> Unit = { key -> if (key in expanded) expanded.remove(key) else expanded.add(key) }
    // Per pane, and dropped with it: what bounds how many decoded images this client is holding
    // is that leaving the pane lets go of all of them.
    val attachments = rememberAttachmentStore(pane.id)
    val listState = rememberLazyListState()
    val rows = remember(pane.revision, query) { transcriptRows(turns, query) }
    // Read once per revision rather than ticked: the ages on a transcript that is being written
    // refresh with every frame the node sends, and a transcript nobody is writing has ages that
    // were true when the reader arrived.
    val now = remember(pane.revision) { wallClockMillis() }
    val hits = remember(rows, query) { searchHits(rows, query) }
    val leading = if (pane.convoMore) 1 else 0

    val scope = rememberCoroutineScope()
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

    // Whether the reader is still standing on the end, which only the reader can give up: they
    // scroll back and it is theirs to keep, they return to the end and the transcript has it
    // again. Read off `lastScrolledBackward` rather than off `isScrollInProgress`, because the
    // list is scrolled forward for reasons that are not the reader — a control near the bottom
    // edge bringing itself into view is one — and being carried towards the end is not the same
    // as choosing to leave it.
    var following by remember(pane.id) { mutableStateOf(true) }
    LaunchedEffect(listState) {
        snapshotFlow { listState.lastScrolledBackward to listState.canScrollForward }
            .collect { (backward, below) ->
                following = when {
                    !below -> true
                    backward -> false
                    else -> following
                }
            }
    }

    // The transcript follows its own end, and it is aimed at that end again whenever the end can
    // have moved without the reader asking it to. Two things move it, and neither is a count of
    // anything: the node writing — a turn appended, a live answer revised, a page of older turns
    // prepended — and the list's own box changing height. The second is the one every earlier fix
    // missed. It is the *list's* box and not the pane's, because the question card below takes a
    // band off the top of the list and off nothing else. `PaneScreenMobile` lays the transcript
    // out under a *guessed* chrome height and replaces the guess with the header's own the moment
    // `onGloballyPositioned` reports it, which is after the transcript's first layout; the
    // keyboard then takes the bottom of the window over a quarter of a second. A lazy list
    // anchors on its *first* visible item, so a box that loses height pushes its end below the
    // fold and leaves it there. Measured in the harness: a
    // chrome guess 52 dp short leaves the last line 52 dp below the fold, on every open.
    //
    // Deliberately *not* keyed on how tall the content measures. A tool card unfolding and a
    // picture decoding are the reader's own doing and must leave the transcript where they put it
    // (AttachmentScrollTest), which is why this is keyed on the turns and the box rather than on
    // anything the list reports about its own extent.
    //
    // `requestScrollToItem`, not `scrollToItem`: the suspending one waits for the list's first
    // layout, and a wait that outlives the composition is resumed with nowhere to go — which in
    // this suite surfaces as an uncaught exception charged to whichever test runs next.
    var viewport by remember { mutableStateOf(0) }
    LaunchedEffect(turns, viewport) {
        if (following && rows.isNotEmpty()) {
            listState.requestScrollToItem(rows.lastIndex + leading, END_OF_THE_ITEM)
        }
    }

    // The bar is 53 dp of a rotated phone's 117 dp of conversation — its search glyph carries a
    // landscape touch target — and it yields all of it while the keys are up, because a reader
    // with a keyboard open is writing rather than reading a turn count. Unless the search field is
    // the thing holding the keyboard, in which case the bar *is* what is being typed into.
    val keyboardOpen = LocalSafeArea.current.ime > 0.dp

    // The question card floats over the top of the transcript rather than standing in the column
    // with it, because the column is where the reply box is measured and a rotated phone with the
    // keys up has no room to spend on a card there (commit cca3022, ComposerInsetTest). An
    // overlay that takes nothing back occludes instead: the transcript scrolled under it and the
    // card sliced through whatever line was behind its top edge. So the transcript is handed that
    // much of its own box, which both reserves the band and clips it — measured, never named,
    // because the card wraps onto a different number of rows for every question it carries.
    val question = if (io.readOnly) null else pane.pending?.takeIf { it.question != null }
    var strip by remember { mutableStateOf(0) }
    val density = LocalDensity.current
    Column(modifier.fillMaxSize().background(tokens.color.bg)) {
        if (searching || !keyboardOpen) {
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
                adrift = !following && rows.isNotEmpty(),
                onEnd = { scope.launch { listState.scrollToItem(rows.lastIndex + leading, END_OF_THE_ITEM) } },
            )
        }

        // Everything the transcript shows, under one selection: the turns, the prompt the agent
        // is waiting on, and the line that stands in for both when there is nothing yet. A lazy
        // list only holds the items it has composed, so a drag reaches as far as the reader has
        // scrolled and no further — the alternative is laying an unbounded transcript out at once.
        SelectionContainer(Modifier.weight(1f)) {
            Box(Modifier.fillMaxSize()) {
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
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(top = if (question == null) 0.dp else with(density) { strip.toDp() })
                        .onSizeChanged { viewport = it.height },
                    contentPadding = androidx.compose.foundation.layout.PaddingValues(
                        start = 16.dp, end = 16.dp, top = 12.dp, bottom = 16.dp,
                    ),
                    verticalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    if (pane.convoMore) {
                        item(key = "older") {
                            DisableSelection {
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
                    }
                    items(rows, key = { it.key }) { row ->
                        when (row) {
                            is TranscriptRow.One ->
                                TurnView(row.turn, query, expanded, toggle, attachments = attachments, now = now)
                            is TranscriptRow.Run ->
                                ToolRunCard(row, row.key in expanded, { toggle(row.key) }) {
                                    for (turn in row.turns) {
                                        TurnView(turn, query, expanded, toggle, attachments = attachments, now = now)
                                    }
                                }
                        }
                    }
                }
                question?.let {
                    PendingStrip(
                        it,
                        onAnswer = { key -> io.send(ClientMsg.Answer(pane.id, key)) },
                        modifier = Modifier.onSizeChanged { size -> strip = size.height },
                    )
                }
            }
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
    adrift: Boolean,
    onEnd: () -> Unit,
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
            // Only while it would do something, and to the *left* of search: a control that comes
            // and goes must not move the one that is always there out from under a thumb. It costs
            // the rotated bar width rather than height, which is the axis that bar is short of.
            if (adrift) {
                GlyphTarget(
                    ConversationIcons.toEnd, "Go to the end of the transcript", tokens.color.accent,
                    onEnd, target = LANDSCAPE_TOUCH, glyph = 15.dp,
                )
            }
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
