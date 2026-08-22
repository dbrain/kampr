package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.BorderSpec
import dev.kampr.shared.theme.Kampr

// error.code is an open string: an unrecognised code still shows its message.
@Composable
internal fun BoxScope.ErrorStrip(message: String, code: String, onDismiss: () -> Unit) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.md)
    val spoken = "${message.ifBlank { code }} ($code). Activate to dismiss."
    Row(
        Modifier
            .align(Alignment.TopCenter)
            .padding(12.dp)
            .background(tokens.color.blockedBg, shape)
            .edge(BorderSpec(1.dp, tokens.color.blocked), shape)
            .announce(spoken, urgent = true)
            .touchable()
            .action(spoken, onDismiss, shape)
            .padding(horizontal = 14.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(9.dp),
    ) {
        IconGlyph(KamprIcons.warning, 14.dp, tokens.color.blocked)
        KText(message.ifBlank { code }, tokens.type.caption, tokens.color.text)
        KText(code, tokens.type.meta, tokens.color.mute)
    }
}

// The same shape as the error strip in a tone that is not a refusal: enrolling a passkey succeeds
// silently otherwise, which on the one screen about credentials is indistinguishable from nothing
// having happened.
@Composable
internal fun BoxScope.NoteStrip(message: String, onDismiss: () -> Unit, accent: Color = Kampr.tokens.color.done) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.md)
    val spoken = "$message Activate to dismiss."
    Row(
        Modifier
            .align(Alignment.TopCenter)
            .padding(12.dp)
            .background(tokens.color.surface2, shape)
            .edge(BorderSpec(1.dp, accent), shape)
            .announce(spoken)
            .touchable()
            .action(spoken, onDismiss, shape)
            .padding(horizontal = 14.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(9.dp),
    ) {
        Mark(accent, MarkShape.Bar, 7.dp)
        KText(message, tokens.type.caption, tokens.color.text, maxLines = 3)
    }
}

// A stored enrolment the node will not have. Nothing here is retryable by waiting, which is exactly
// what "reconnecting in 12s" over a cached herd told the operator for as long as they left it up —
// so this says what happened and leads to the one screen that can fix it.
@Composable
internal fun BoxScope.RefusedNotice(message: String, onPair: () -> Unit) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.md)
    val spoken = "$message Activate to pair this device again."
    Row(
        Modifier
            .align(Alignment.TopCenter)
            .padding(12.dp)
            .background(tokens.color.blockedBg, shape)
            .edge(BorderSpec(1.dp, tokens.color.blocked), shape)
            .announce(spoken, urgent = true)
            .touchable()
            .action(spoken, onPair, shape)
            .padding(horizontal = 14.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(9.dp),
    ) {
        IconGlyph(KamprIcons.warning, 14.dp, tokens.color.blocked)
        KText(message, tokens.type.caption, tokens.color.text, maxLines = 3)
        KText("pair again", tokens.type.meta, tokens.color.blocked)
    }
}

// A device whose permissions moved under it. Not a refusal — nothing the operator did failed —
// but it is the whole of why the buttons went away or came back, and a change nobody announced
// is one that gets discovered by pressing something that no longer works.
@Composable
fun BoxScope.RoleNotice(message: String, onDismiss: () -> Unit) =
    NoteStrip(message, onDismiss, Kampr.tokens.color.accent)
