package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.absolutePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.BorderSpec
import dev.kampr.shared.theme.Kampr

// A strip is not inside any screen — it floats over all of them, so nothing else pays its insets
// for it. Left at zero it was drawn under the status bar and behind the punch-hole, which on a
// pixel_6 is the whole of where a strip aligned to the top of the window lands.
//
// The width bound is what lets a message be longer than a sentence: the one path in this app that
// produces a real explanation produces several lines of it, and a strip that hugs its content
// makes those lines as wide as the window and then clips them.
private val STRIP_MAX_WIDTH = 520.dp

// Enough for the config lines the passkey diagnosis hands over. A cap rather than none because a
// node's own text arrives here too, and a strip that fills the screen has taken it over.
private const val STRIP_MAX_LINES = 12

@Composable
private fun BoxScope.Strip(
    background: Color,
    border: Color,
    spoken: String,
    urgent: Boolean,
    onActivate: () -> Unit,
    content: @Composable RowScope.() -> Unit,
) {
    val tokens = Kampr.tokens
    val safe = LocalSafeArea.current
    val shape = RoundedCornerShape(tokens.radii.md)
    Row(
        Modifier
            .align(Alignment.TopCenter)
            .absolutePadding(left = 12.dp + safe.left, top = 12.dp + safe.top, right = 12.dp + safe.right)
            .widthIn(max = STRIP_MAX_WIDTH)
            .background(background, shape)
            .edge(BorderSpec(1.dp, border), shape)
            .announce(spoken, urgent = urgent)
            .touchable()
            .action(spoken, onActivate, shape)
            .padding(horizontal = 14.dp, vertical = 10.dp),
        verticalAlignment = Alignment.Top,
        horizontalArrangement = Arrangement.spacedBy(9.dp),
        content = content,
    )
}

// error.code is an open string: an unrecognised code still shows its message.
@Composable
internal fun BoxScope.ErrorStrip(message: String, code: String, onDismiss: () -> Unit) {
    val tokens = Kampr.tokens
    val spoken = "${message.ifBlank { code }} ($code). Activate to dismiss."
    Strip(tokens.color.blockedBg, tokens.color.blocked, spoken, urgent = true, onActivate = onDismiss) {
        IconGlyph(KamprIcons.warning, 14.dp, tokens.color.blocked, Modifier.padding(top = 2.dp))
        KText(
            message.ifBlank { code },
            tokens.type.caption,
            tokens.color.text,
            Modifier.weight(1f),
            maxLines = STRIP_MAX_LINES,
        )
        KText(code, tokens.type.meta, tokens.color.mute, Modifier.padding(top = 2.dp))
    }
}

// The same shape as the error strip in a tone that is not a refusal: enrolling a passkey succeeds
// silently otherwise, which on the one screen about credentials is indistinguishable from nothing
// having happened.
@Composable
internal fun BoxScope.NoteStrip(message: String, onDismiss: () -> Unit, accent: Color = Kampr.tokens.color.done) {
    val tokens = Kampr.tokens
    val spoken = "$message Activate to dismiss."
    Strip(tokens.color.surface2, accent, spoken, urgent = false, onActivate = onDismiss) {
        Mark(accent, MarkShape.Bar, 7.dp, Modifier.padding(top = 5.dp))
        KText(message, tokens.type.caption, tokens.color.text, Modifier.weight(1f), maxLines = STRIP_MAX_LINES)
    }
}

// A stored enrolment the node will not have. Nothing here is retryable by waiting, which is exactly
// what "reconnecting in 12s" over a cached herd told the operator for as long as they left it up —
// so this says what happened and leads to the one screen that can fix it.
@Composable
internal fun BoxScope.RefusedNotice(message: String, onPair: () -> Unit) {
    val tokens = Kampr.tokens
    val spoken = "$message Activate to pair this device again."
    Strip(tokens.color.blockedBg, tokens.color.blocked, spoken, urgent = true, onActivate = onPair) {
        IconGlyph(KamprIcons.warning, 14.dp, tokens.color.blocked, Modifier.padding(top = 2.dp))
        KText(message, tokens.type.caption, tokens.color.text, Modifier.weight(1f), maxLines = STRIP_MAX_LINES)
        KText("pair again", tokens.type.meta, tokens.color.blocked, Modifier.padding(top = 2.dp))
    }
}

// A device whose permissions moved under it. Not a refusal — nothing the operator did failed —
// but it is the whole of why the buttons went away or came back, and a change nobody announced
// is one that gets discovered by pressing something that no longer works.
@Composable
fun BoxScope.RoleNotice(message: String, onDismiss: () -> Unit) =
    NoteStrip(message, onDismiss, Kampr.tokens.color.accent)
