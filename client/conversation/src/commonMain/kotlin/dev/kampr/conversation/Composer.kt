package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.absolutePadding
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
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
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import dev.kampr.shared.platform.LocalHardKeyboard
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.TOUCH
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.edge
import dev.kampr.shared.ui.edgeTop
import dev.kampr.shared.ui.named
import dev.kampr.shared.ui.readingOrder

// Herdr's send_text takes a JSON string, and a carriage return is what a harness reads as submit.
// It goes as its own message so a harness that debounces sees the text settle before the newline.
fun replyMessages(paneId: String, text: String): List<ClientMsg> =
    listOf(ClientMsg.InputText(paneId, text), ClientMsg.InputText(paneId, "\r"))

// A plain multi-line text box on purpose: the native keyboard's own dictation button then works,
// which no custom input surface can offer.
@Composable
fun Composer(agent: String?, enabled: Boolean, onSend: (String) -> Unit, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    // The last bar on the pane whenever the conversation is showing, and the pane is the one screen
    // the scaffold does not pad — so in landscape, where no tab bar sits under it, the reply box was
    // the thing the gesture handle landed on and the send button the thing a rotated navigation bar
    // covered. Zero on a portrait phone, where the tabs below already hold the edge.
    val safe = LocalSafeArea.current
    var value by remember { mutableStateOf(TextFieldValue()) }
    // A fixed radius and not `pill`, which is 999 dp and therefore always half the height of
    // whatever it is put on. That reads as a chip on one line of reply and as an oval by four,
    // which is the shape this box spends most of its life in. The send button beside it keeps the
    // pill, because a circle is what it is meant to be at any size.
    val field = RoundedCornerShape(tokens.radii.lg)
    val ready = enabled && value.text.isNotBlank()
    val hard = LocalHardKeyboard.current

    fun submit() {
        if (!ready) return
        onSend(value.text.trimEnd())
        value = TextFieldValue()
    }

    // Written rather than passed through, because the field does not agree with itself about it:
    // shift and return inserts a line on its own and alt and return inserts nothing at all. Doing
    // both here is what makes the two modifiers the same key to whoever is holding one.
    fun newline() {
        val at = value.selection
        value = TextFieldValue(value.text.replaceRange(at.min, at.max, "\n"), TextRange(at.min + 1))
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

    Row(
        modifier
            .fillMaxWidth()
            .background(tokens.color.bar)
            .edgeTop()
            .readingOrder(1f)
            .absolutePadding(
                left = 12.dp + safe.left,
                top = 10.dp,
                right = 12.dp + safe.right,
                bottom = 14.dp + safe.bottom,
            ),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(9.dp),
    ) {
        Box(
            Modifier
                .weight(1f)
                .background(tokens.color.surface, field)
                .edge(tokens.card, field)
                .padding(horizontal = 16.dp, vertical = 12.dp),
        ) {
            if (value.text.isEmpty()) {
                KText(
                    if (enabled) "Reply to ${agent ?: "the agent"}…" else "read-only device",
                    tokens.type.body,
                    tokens.color.mute,
                )
            }
            BasicTextField(
                value = value,
                onValueChange = { value = it },
                enabled = enabled,
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
