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
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.layout.Measurable
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.AgentStatus
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.statusOf
import dev.kampr.shared.net.wallClockMillis
import dev.kampr.shared.platform.LocalReduceMotion
import dev.kampr.shared.platform.PickedFile
import dev.kampr.shared.platform.filePickAvailable
import dev.kampr.shared.platform.PastedFiles
import dev.kampr.shared.platform.pickFile
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LabelText
import dev.kampr.shared.ui.LocalConnectionStatus
import dev.kampr.shared.ui.answering
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.PaneIo
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
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.talks
import kotlin.io.encoding.Base64
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

// Compose can aim a lazy list at an item's *top* and at nothing else, so a transcript that ends
// on a long answer opened at the start of that answer — which read as "somewhere above the
// bottom", by however tall the newest message happened to be, or as the top outright when the
// newest message was the whole screen. An offset past the end of any item is measured against the
// end of the list, which is the only thing that means "the bottom" here.
private const val END_OF_THE_ITEM = Int.MAX_VALUE

@Composable
fun ConversationView(
    pane: PaneState,
    info: PaneInfo?,
    modifier: Modifier = Modifier,
    // Hoisted only so a harness can seed it. Both gestures that fill it in are unreachable from a
    // test on this platform — a desktop's file picker is a modal system dialog, and a desktop has
    // no page-wide paste event at all — so the state has to be reachable from outside or the one
    // transition that clears it can never be driven through the real view.
    handover: MutableState<Handover> = remember(pane.id) { mutableStateOf(Handover.Idle) },
    // Injected only so an artboard can be drawn against a fixed one — a picture whose stopwatch
    // reads the real clock is a different picture every time it is rendered.
    clock: () -> Double = ::wallClockMillis,
) {
    val tokens = Kampr.tokens
    val io = LocalPaneIo.current

    // `converses` is the node saying it serves this harness's transcripts at all; `hasConversation`
    // is whether it has read one yet. An agent that was launched a moment ago and has not been
    // prompted answers yes and no, and reading only the second told the operator this node had no
    // adapter for the harness — under a tab that is only there because it has one. Nothing needs
    // passing through and reattaching: a reply is a write to the pane's own PTY, and the transcript
    // catches up the moment the harness writes its first record.
    if (info != null && !info.talks) {
        AbsentConversation(info, modifier) { io.show(PaneView.Terminal) }
        return
    }

    // A live preview is withdrawn by arriving again with no blocks, so an empty turn is a turn
    // that is no longer there.
    //
    // The queue goes on the end, after every record: the harness has not started on any of it yet,
    // so it is the newest thing anybody has said to this pane.
    val turns = remember(pane.revision) { pane.turns.filter { it.isVisible() } + queuedTurns(pane.facets) }
    var query by remember { mutableStateOf("") }
    var searching by remember { mutableStateOf(false) }
    var focus by remember { mutableStateOf(0) }
    val expanded = remember { mutableStateListOf<String>() }
    val toggle: (String) -> Unit = { key -> if (key in expanded) expanded.remove(key) else expanded.add(key) }
    // Per pane, and dropped with it: what bounds how many decoded images this client is holding
    // is that leaving the pane lets go of all of them.
    val attachments = rememberAttachmentStore(pane.id)
    val listState = rememberLazyListState()
    // `derivedStateOf` rather than a keyed `remember`: what is put away is a snapshot list, and
    // the rows are a function of it — a collapsed reply is one row where an open one is a dozen.
    val rows by remember(pane.revision, query) {
        derivedStateOf { transcriptRows(turns, query, expanded) }
    }
    // Ticked, and it has to be: the stamps used to be read once per revision, so a transcript
    // nobody was writing kept whatever ages were true when the reader arrived and the newest
    // message read "now" for as long as the pane stayed open. What the stamps carry now is a time
    // of day (#285), which does not move on its own — this is what moves the *bucket* it is drawn
    // in, from a bare clock to a weekday to a date, and what carries the age a zoneless stamp
    // still falls back to. A minute is finer than any of those need.
    var now by remember { mutableStateOf(clock()) }
    LaunchedEffect(Unit) {
        while (true) {
            delay(60_000)
            now = clock()
        }
    }
    val working = info != null && statusOf(info) == AgentStatus.Working
    val newest = (rows.lastOrNull { it is TranscriptRow.Head } as? TranscriptRow.Head)?.reply
    val tail = newest?.takeIf { working || it.live }
    // What a reader is looking at, when it is not the conversation as it stands. The grid beside
    // this reattaches to a stream the registry held open, so it is current the moment the pane
    // opens; the transcript has to be resolved, folded and paged first, and what is on screen
    // until then is a memory (#393).
    val catchingUp = catchingUp(
        LocalConnectionStatus.current,
        confirmed = pane.convoConfirmed && !pane.stale,
        drawn = rows.isNotEmpty(),
    )
    // The progress line is an item of its own, so the end of the list is one past the last row
    // whenever it is showing — and the end of the list is what "follow the end" aims at. The
    // read-up-to line below it needs no place here: the aim is `Int.MAX_VALUE` into whichever item
    // it names, so it clamps to the foot of the content whatever is standing there.
    val trailing = if (tail == null) 0 else 1
    // The progress line is a piece of the newest reply's box, and it is at the foot of the
    // transcript rather than at the head of that reply because the transcript follows its own end:
    // the end is where the reader's eye already is, and the head of a reply eleven minutes long is
    // far above the fold, which is the one place a progress line is no use.
    val shown = if (tail == null) rows else rows + TranscriptRow.Working(tail)
    val stamps = remember(shown, now) { stepStamps(shown, now) }
    val hits = remember(rows, query) { searchHits(rows, query) }
    val leading = if (pane.convoMore) 1 else 0

    val scope = rememberCoroutineScope()
    // A node refuses a paste with an error naming this pane — too large, not base64, nowhere to
    // write — and that error is quiet everywhere else by design, so this is the only place it can
    // be said.
    LaunchedEffect(pane.refusal) { handover.value = handoverAfter(handover.value, pane.refusal) }
    // The same handover as the attach button's, reached by the gesture a desk actually uses. A
    // clipboard with no file on it never gets here, so pasting words into the reply box is
    // untouched.
    PastedFiles(!io.readOnly) { picked -> handover.value = handoverOf(pane, io, picked) }
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
    LaunchedEffect(turns, viewport, trailing) {
        if (following && rows.isNotEmpty()) {
            listState.requestScrollToItem(rows.lastIndex + leading + trailing, END_OF_THE_ITEM)
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
    // Keyed on the rows, and it has to be: a `remember` that outlives them closes over the list it
    // was created with, and on a real open that list is the empty one the screen composes before
    // the transcript arrives. The bar then had nothing to name for the life of the pane, while
    // three harness tests that handed the view a finished transcript all said it worked.
    val pinned by remember(shown, leading) { derivedStateOf { pinnedBlock(listState, shown, leading) } }
    var strip by remember { mutableStateOf(0) }
    // The band under the transcript belongs to a strip that is standing there, and read through
    // the list rather than off the last measurement: a composable that returns before it draws
    // anything is never measured again, so a strip that has gone reports the size it had when it
    // was there for ever.
    var footPx by remember { mutableStateOf(0) }
    val foot = if (pane.facets.running.isEmpty()) 0 else footPx
    val density = LocalDensity.current
    // Over the whole pane and outside the transcript's own column, because that is what a picture
    // opened to be looked at needs — and because the card that opened it is inside a lazy list,
    // which composes it away the moment it scrolls off.
    attachments.viewing?.let { att ->
        when (val open = attachments.state(att.id)) {
            is AttachmentState.Shown -> {
                ImageViewer(
                    image = open.image,
                    headline = headlineOf(att),
                    detail = detailOf(att),
                    onSave = { scope.launch { attachments.save(att) } },
                    onClose = { attachments.close() },
                    saved = attachments.saved(att.id),
                    modifier = modifier,
                )
                return
            }
            is AttachmentState.Text -> {
                FileViewer(
                    att = att,
                    text = open.text,
                    attachments = attachments,
                    saved = attachments.saved(att.id),
                    onSave = { scope.launch { attachments.save(att) } },
                    onClose = { attachments.close() },
                    modifier = modifier,
                )
                return
            }
            // Dropped out of the held set while it was open. Nothing reaches this today — the only
            // thing that evicts is another picture being opened, and none can be while this one is
            // — but the alternative to saying so is a blank pane with no way off it. Cleared in an
            // effect rather than here: writing state during composition is a loop, not a fix.
            else -> LaunchedEffect(att.id) { attachments.close() }
        }
    }

    // A stack, not a flag: `depth` says a launched conversation can launch one of its own, so
    // going in twice and coming back once has to mean something. Dropped with the pane.
    val opened = remember(pane.id) { mutableStateListOf<Block.Sub>() }
    val openSub: (Block.Sub) -> Unit = { sub ->
        // Asked for every time it is opened rather than only the first: the file is appended to
        // while the agent runs, and the page is `fresh` by rule, so a second ask is how a running
        // subagent's latest step arrives.
        io.send(ClientMsg.ConvoSub(pane.id, sub.id))
        opened.add(sub)
    }

    CompositionLocalProvider(LocalOpenSub provides openSub) {
        opened.lastOrNull()?.let { sub ->
            SubConversationView(
                sub = sub,
                state = pane.subOrNull(sub.id),
                agent = info?.agent,
                now = now,
                onBack = { opened.removeAt(opened.lastIndex) },
                onOlder = { cursor -> io.send(ClientMsg.ConvoSub(pane.id, sub.id, cursor)) },
                modifier = modifier,
                clock = clock,
            )
            return@CompositionLocalProvider
        }

        ConversationColumn(modifier.fillMaxSize().background(tokens.color.bg), bar = {
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
            }, transcript = {
        SelectionContainer(Modifier.fillMaxSize()) {
                Box(Modifier.fillMaxSize()) {
                    if (turns.isEmpty()) {
                        KText(
                            if (info?.hasConversation == false) {
                                "nothing written down yet \u2014 what you send is what starts it"
                            } else {
                                "waiting for the transcript"
                            },
                            tokens.type.caption,
                            tokens.color.mute,
                            Modifier.align(Alignment.Center),
                        )
                    }
                    LazyColumn(
                        state = listState,
                        modifier = Modifier
                            .fillMaxSize()
                            .padding(
                                top = if (question == null) 0.dp else with(density) { strip.toDp() },
                                bottom = with(density) { foot.toDp() },
                            )
                            .onSizeChanged { viewport = it.height },
                        contentPadding = androidx.compose.foundation.layout.PaddingValues(
                            start = 16.dp, end = 16.dp, top = 12.dp, bottom = 16.dp,
                        ),
                        // None. A block is one box drawn a piece at a time, and a gap between the
                        // pieces is a gap in the box — the foot of each block pays the space that
                        // separates it from the next one, outside its own paint.
                        verticalArrangement = Arrangement.spacedBy(0.dp),
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
                        itemsIndexed(shown, key = { _, row -> row.key }) { at, _ ->
                            TranscriptRowView(
                                shown, at, stamps.getOrNull(at), query, expanded, toggle,
                                attachments = attachments, now = now, agent = info?.agent, clock = clock,
                            )
                        }
                        catchingUp?.let { said ->
                            item(key = "read-up-to") {
                                DisableSelection { CatchingUpLine(said) }
                            }
                        }
                    }
                    // The header of whatever the reader is standing in the middle of, pinned where
                    // that turn's own header used to be, so putting a long message away never means
                    // scrolling back up to find the chevron that does it. It sits under the question
                    // card for the same reason the list is inset by it: the card is the one thing on
                    // this screen that outranks the transcript.
                    pinned?.let { at ->
                        PinnedBlockBar(
                            at,
                            info?.agent,
                            now,
                            onCollapse = collapseKey(at.head)?.let { key ->
                                {
                                    toggle(key)
                                    scope.launch { listState.scrollToItem(at.index + leading) }
                                }
                            },
                            modifier = Modifier
                                .align(Alignment.TopStart)
                                .padding(top = if (question == null) 0.dp else with(density) { strip.toDp() }),
                        )
                    }
                    question?.let {
                        PendingStrip(
                            it,
                            answering = answering(LocalConnectionStatus.current, pane.undelivered),
                            onAnswer = { key -> io.send(ClientMsg.Answer(pane.id, key)) },
                            onSubmit = { io.send(ClientMsg.AnswerSubmit(pane.id)) },
                            modifier = Modifier.onSizeChanged { size -> strip = size.height },
                        )
                    }
                    // Pinned at the foot rather than in the turn that launched it: what is running
                    // now is a fact about now, and the card in the transcript is only findable by
                    // scrolling back to the moment of the launch. Under the question, which is the
                    // one thing on this screen that outranks everything.
                    //
                    // Measured, and the list handed that much of its own box, for the reason the
                    // question card above is: an overlay that takes nothing back does not sit
                    // above the transcript, it sits *on* it — the last turn was under the strip
                    // for as long as anything was running. The list's 16 dp of foot survives it
                    // and becomes the gap between the two.
                    RunningStrip(
                        pane.facets.running,
                        Modifier.align(Alignment.BottomStart).onSizeChanged { size -> footPx = size.height },
                        open = RUNNING_OPEN in expanded,
                        onFold = { toggle(RUNNING_OPEN) },
                        clock = clock,
                    )
                }
            }

        }, composer = {
            Composer(
                agent = info?.agent,
                enabled = !io.readOnly,
                answering = answering(LocalConnectionStatus.current, pane.undelivered),
                onSend = { text ->
                    replyMessages(pane.id, text).forEach(io::send)
                    handover.value = handoverAfterSend(handover.value)
                },
                draft = pane.draft,
                onDraft = { pane.draft = it },
                onAttach = if (io.readOnly || !filePickAvailable) null else {
                    {
                        scope.launch {
                            val picked = pickFile() ?: return@launch
                            handover.value = handoverOf(pane, io, picked)
                        }
                    }
                },
                handover = handover.value,
                desk = pane.desk,
                // The keystroke is the node's measurement and arrives with the line, so nothing
                // here decides what empties a box on a machine it has never seen. It goes as
                // ordinary `input`, which is the path a device that may not type is already
                // refused on.
                onTakeOver = { line -> line.clear?.let { io.send(ClientMsg.InputText(pane.id, it)) } },
            )
        })
    }
}

// The reply box is the one child of this column that must survive a short window.
//
// **A column measures its unweighted children in index order against what the ones before them
// left**, so the reply box — last in reading order — was last in the queue for room that had
// already gone: rotated with the keys up it measured 0 dp tall and there was nothing to type into
// (#319). The bar above it is a count and a search field, and the transcript is scrollable; both
// can lose height and still be themselves, and the reply box cannot.
//
// So the three are measured in priority order — the composer at its natural height, then the bar
// with what is left, then the transcript with the remainder, which may legitimately be nothing —
// and placed afterwards in the order a reader meets them. The previous shape was a plain `Column`
// with the transcript weighted, which is the same thing for every window tall enough to hold all
// three and silently the wrong thing for every window that is not.
@Composable
internal fun ConversationColumn(
    modifier: Modifier,
    bar: @Composable () -> Unit,
    transcript: @Composable () -> Unit,
    composer: @Composable () -> Unit,
) {
    Layout(contents = listOf(composer, bar, transcript), modifier = modifier) { (composerM, barM, transcriptM), constraints ->
        val width = constraints.maxWidth
        val room = constraints.maxHeight
        fun slot(measurables: List<Measurable>, height: Int, exact: Boolean = false) =
            measurables.map {
                it.measure(
                    Constraints(
                        minWidth = width,
                        maxWidth = width,
                        minHeight = if (exact) height else 0,
                        maxHeight = height,
                    )
                )
            }

        val composed = slot(composerM, room)
        val afterComposer = (room - composed.sumOf { it.height }).coerceAtLeast(0)
        val barred = slot(barM, afterComposer)
        val left = (afterComposer - barred.sumOf { it.height }).coerceAtLeast(0)
        val body = slot(transcriptM, left, exact = true)

        layout(width, room) {
            var y = 0
            barred.forEach { it.place(0, y); y += it.height }
            body.forEach { it.place(0, y); y += it.height }
            composed.forEach { it.place(0, y); y += it.height }
        }
    }
}

// The ceiling the node applies, applied here as well. Sending eight megabytes up a phone link to
// be refused at the other end is a minute of somebody's tethering spent on a certain no.
internal const val MOST_BYTES_HANDED_OVER = 8 * 1024 * 1024

// The node answers a paste it will not take with an error naming this pane — too large, not
// base64, nowhere to write — and that error is deliberately quiet everywhere else, so the composer
// is the only place it can be said. Nothing else the composer does produces one, and the pane's
// refusal is cleared before the bytes go, so what lands next is the answer to this.
// The line is a statement about the draft — *its path is typed in* — and the reply going is what
// makes it false. Nothing used to take it down: the state had no transition back to idle at all,
// so the green line stood over the composer for the life of the pane, still naming a screenshot
// three messages ago. (The terminal view has the same missing transition, cleared there by the
// pane's next input.)
//
// A refusal is not a statement about the draft. It is the node's only report that a file never
// arrived — quiet everywhere else by design — and an error the operator has not read is not
// something to sweep away with an action that has nothing to do with it. The next handover clears
// it, which is the press that was going to fix it anyway.
internal fun handoverAfterSend(handover: Handover): Handover =
    if (handover is Handover.Refused) handover else Handover.Idle

internal fun handoverAfter(handover: Handover, refusal: String?): Handover =
    if (refusal != null && handover is Handover.Sent) Handover.Refused(refusal) else handover

internal fun handoverOf(pane: PaneState, io: PaneIo, picked: PickedFile): Handover {
    val name = picked.name?.takeIf { it.isNotBlank() } ?: "the file"
    if (picked.bytes.size > MOST_BYTES_HANDED_OVER) {
        return Handover.Refused("$name is larger than the 8 MiB a node will take.")
    }
    pane.clearRefusal()
    io.send(ClientMsg.Paste(pane.id, Base64.encode(picked.bytes), picked.name))
    return Handover.Sent(name)
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
