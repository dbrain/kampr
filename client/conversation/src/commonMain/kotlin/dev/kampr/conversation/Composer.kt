package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.absolutePadding
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.foundation.text.input.clearText
import androidx.compose.foundation.text.input.rememberTextFieldState
import androidx.compose.foundation.text.input.InputTransformation
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEvent
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.isAltPressed
import androidx.compose.ui.input.key.isCtrlPressed
import androidx.compose.ui.input.key.isMetaPressed
import androidx.compose.ui.input.key.isShiftPressed
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.DeskLine
import dev.kampr.shared.platform.LocalHardKeyboard
import dev.kampr.shared.platform.acceptsPastedFiles
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.ui.GlyphTarget
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.TOUCH
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.announce
import dev.kampr.shared.ui.edge
import dev.kampr.shared.ui.edgeTop
import dev.kampr.shared.ui.named
import dev.kampr.shared.ui.readingOrder

// Herdr's send_text takes a JSON string, and a carriage return is what a harness reads as submit.
// It goes as its own message so a harness that debounces sees the text settle before the newline.
fun replyMessages(paneId: String, text: String): List<ClientMsg> =
    listOf(ClientMsg.InputText(paneId, text), ClientMsg.InputText(paneId, "\r"))

// Android hangs a text selection handle 25 dp below the line it grips, and hangs the start handle
// the same distance to the left of the character the selection begins at. They are windows of
// their own, so nothing here can clip them or move them — the only thing this bar can do is not
// stand where they land. It did not: the strip under the field came to 26 dp and the strip beside
// it to 28, each of them the sum of a padding chosen for the field's shape and one chosen for the
// bar's, and on a rotated pane — the one posture where the keys are directly under the composer —
// that cleared the handle by a single dp. The handle is the floor of both paddings now, so
// retuning either of them for how it looks cannot put a handle under the keys.
private val SELECTION_HANDLE = 25.dp
private val FIELD_INSET_X = 16.dp
private val FIELD_INSET_Y = 12.dp

// Where a file the operator handed over has got to. The node writes the bytes on the pane's own
// machine and types the path in, so "sent" is the whole of the success — there is no upload to
// watch — and a refusal comes back as an error naming this pane.
sealed interface Handover {
    data object Idle : Handover
    data class Going(val name: String) : Handover
    data class Sent(val name: String) : Handover
    data class Refused(val reason: String) : Handover
}

// A plain multi-line text box on purpose: the native keyboard's own dictation button then works,
// which no custom input surface can offer.
@Composable
fun Composer(
    agent: String?,
    enabled: Boolean,
    onSend: (String) -> Unit,
    modifier: Modifier = Modifier,
    onAttach: (() -> Unit)? = null,
    handover: Handover = Handover.Idle,
    draft: String = "",
    onDraft: (String) -> Unit = {},
    desk: DeskLine? = null,
    onTakeOver: (DeskLine) -> Unit = {},
) {
    val tokens = Kampr.tokens
    // The last bar on the pane whenever the conversation is showing, and the pane is the one screen
    // the scaffold does not pad — so in landscape, where no tab bar sits under it, the reply box was
    // the thing the gesture handle landed on and the send button the thing a rotated navigation bar
    // covered. Zero on a portrait phone, where the tabs below already hold the edge.
    val safe = LocalSafeArea.current
    // **Seeded from the pane, not owned here.** A `remember` of its own died with the composable,
    // and switching to the terminal view is exactly what takes this out of the composition — so a
    // half-written reply was lost to a glance at the pane it was about. The caret goes to the end,
    // which is where somebody returning to their own sentence wants it.
    //
    // A plain `remember` and **not** `rememberTextFieldState`, whose store is `rememberSaveable`:
    // this composable is not keyed by pane, so a saved buffer restored into it is the previous
    // pane's half-written reply appearing in this one's box.
    val value = remember { TextFieldState(draft, TextRange(draft.length)) }
    // **Reported inside the edit, and it has to be.** The two other places to put this are both a
    // frame late — a `snapshotFlow` in a `LaunchedEffect` and a `SideEffect` on composition alike
    // — and the frame they are late by is the one where the operator switches to the terminal,
    // taking this composable and its unreported draft out of the composition. That is the loss
    // this seam exists to prevent, so the report happens where the old `onValueChange` did.
    //
    // An input transformation does not see `state.edit`, which is deliberate elsewhere
    // (`FieldTextInput`) and is why the three edits this file makes report for themselves.
    val report = remember(onDraft) { InputTransformation { onDraft(asCharSequence().toString()) } }
    val typed = value.text.toString()
    // A fixed radius and not `pill`, which is 999 dp and therefore always half the height of
    // whatever it is put on. That reads as a chip on one line of reply and as an oval by four,
    // which is the shape this box spends most of its life in. The send button beside it keeps the
    // pill, because a circle is what it is meant to be at any size.
    val field = RoundedCornerShape(tokens.radii.lg)
    val ready = enabled && typed.isNotBlank()
    val hard = LocalHardKeyboard.current

    fun submit() {
        if (!ready) return
        onSend(typed.trimEnd())
        value.clearText()
        onDraft("")
    }

    // Written rather than passed through, because the field does not agree with itself about it:
    // shift and return inserts a line on its own and alt and return inserts nothing at all. Doing
    // both here is what makes the two modifiers the same key to whoever is holding one.
    fun newline() {
        value.edit {
            val at = selection.min
            replace(selection.min, selection.max, "\n")
            placeCursorBeforeCharAt(at + 1)
        }
        onDraft(value.text.toString())
    }

    // Return sends, and a modifier with it writes the second line — which is what every agent CLI
    // on the far end of this pane already does, and what anyone with a keyboard expects of a box
    // that has a send button beside it.
    //
    // Only where there is a keyboard to press. On a phone the soft keyboard's return **is** the
    // only newline it offers, so taking it would leave a reply that can never have two lines, and
    // the send button is already on the screen an inch away. `LocalHardKeyboard` reads false on
    // every platform that cannot tell, which is the right side to be wrong on here as well.
    //
    // Both halves of the press are consumed, not just the down that acts. Nothing here is measured
    // to break without it — a key this function has taken over is simply not one to hand back half
    // of, and which half of a return a text field acts on is a platform's business, not this one's.
    fun onReturn(event: KeyEvent): Boolean {
        if (!hard) return false
        if (event.key != Key.Enter && event.key != Key.NumPadEnter) return false
        if (event.isCtrlPressed || event.isMetaPressed) return false
        if (event.type == KeyEventType.KeyDown) {
            if (event.isShiftPressed || event.isAltPressed) newline() else submit()
        }
        return true
    }

    // **The takeover is a press and nothing else.** Switching to this view, opening the pane or
    // reconnecting must never move a character on the far machine: looking at a pane has no side
    // effects, and a write that empties somebody's half-written sentence is the last thing to make
    // an exception of. What it does is *move* the line rather than destroy it — the words land in
    // this box before the pane is asked to let go of them — so the worst a mistimed press can cost
    // is a paste back, and the pane's own harness has an undo for it besides.
    fun takeOver(line: DeskLine) {
        value.edit {
            replace(0, 0, line.text)
            placeCursorBeforeCharAt(length)
        }
        onDraft(value.text.toString())
        onTakeOver(line)
    }

    // On the whole column rather than on the field: a `contentReceiver` on a parent serves every
    // text field under it, and it is a drag-and-drop target in its own right, so a file dropped
    // anywhere on the reply bar lands the same way a pasted one does.
    Column(
        modifier
            .fillMaxWidth()
            .background(tokens.color.bar)
            .acceptsPastedFiles()
            .edgeTop()
            .readingOrder(1f)
    ) {
        DeskStrip(desk, agent, enabled, { desk?.let(::takeOver) })
        HandoverLine(handover, agent)
        Row(
            Modifier
                .fillMaxWidth()
                .absolutePadding(
                    left = (SELECTION_HANDLE - FIELD_INSET_X).coerceAtLeast(12.dp) + safe.left,
                    top = 10.dp,
                    right = 12.dp + safe.right,
                    bottom = (SELECTION_HANDLE - FIELD_INSET_Y).coerceAtLeast(14.dp) + safe.bottom,
                ),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(9.dp),
        ) {
            // An agent over ssh reads a local path perfectly well; it is the terminal's own
            // image-paste protocol that dies. So this hands the bytes to the node, which writes them
            // beside the pane and types the path in. Absent where there is no picker to raise.
            if (onAttach != null && enabled) {
                GlyphTarget(
                    ConversationIcons.attach,
                    "Attach a file for ${agent ?: "the agent"}",
                    tokens.color.dim,
                    onAttach,
                    target = TOUCH,
                    glyph = 17.dp,
                )
            }
            Box(
                Modifier
                    .weight(1f)
                    .background(tokens.color.surface, field)
                    .edge(tokens.card, field)
                    .padding(horizontal = FIELD_INSET_X, vertical = FIELD_INSET_Y),
            ) {
                if (typed.isEmpty()) {
                    KText(
                        if (enabled) "Reply to ${agent ?: "the agent"}…" else "read-only device",
                        tokens.type.body,
                        tokens.color.mute,
                    )
                }
                BasicTextField(
                    state = value,
                    enabled = enabled,
                    inputTransformation = report,
                    modifier = Modifier
                        .fillMaxWidth()
                        .heightIn(max = 96.dp)
                        .onPreviewKeyEvent(::onReturn)
                        .named(if (enabled) "Reply to ${agent ?: "the agent"}" else "Read-only device — replies are refused"),
                    textStyle = tokens.type.body.copy(color = tokens.color.text),
                    cursorBrush = SolidColor(tokens.color.accent),
                )
            }
            val pillShape = RoundedCornerShape(tokens.radii.pill)
            Box(
                Modifier
                    .size(TOUCH)
                    .background(if (ready) tokens.color.accent else tokens.color.raise, pillShape)
                    .action(
                        "Send this reply to ${agent ?: "the agent"}",
                        { submit() },
                        pillShape,
                        enabled = ready,
                    ),
                contentAlignment = Alignment.Center,
            ) {
                IconGlyph(ConversationIcons.send, 18.dp, if (ready) tokens.color.onAccent else tokens.color.mute)
            }
        }
    }
}

@Composable
private fun HandoverLine(handover: Handover, agent: String?) {
    val tokens = Kampr.tokens
    val (words, tone) = when (handover) {
        Handover.Idle -> return
        is Handover.Going -> "sending ${handover.name}" to tokens.color.working
        is Handover.Sent -> "${handover.name} is on ${agent ?: "the agent"}'s machine, and its path is typed in" to
            tokens.color.done
        is Handover.Refused -> handover.reason to tokens.color.blocked
    }
    KText(
        words,
        tokens.type.micro,
        tone,
        Modifier
            .fillMaxWidth()
            .padding(start = 16.dp, end = 16.dp, top = 8.dp)
            .announce(words),
        maxLines = 3,
    )
}
