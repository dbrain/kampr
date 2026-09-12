package dev.kampr.conversation

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.GlyphTarget
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LANDSCAPE_TOUCH
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.touchable
import kotlinx.coroutines.delay

class Copying(val copied: Boolean, val tint: Color, val copy: () -> Unit)

@Composable
fun rememberCopying(text: String): Copying {
    val tokens = Kampr.tokens
    // LocalClipboard replaces this, but its ClipEntry can only be built from a platform-native
    // object, so a plain string still has no common-code path in CMP 1.11.
    @Suppress("DEPRECATION")
    val clipboard = LocalClipboardManager.current
    var copied by remember(text) { mutableStateOf(false) }
    LaunchedEffect(copied) {
        if (copied) {
            delay(1400)
            copied = false
        }
    }
    return Copying(copied, if (copied) tokens.color.done else tokens.color.dim) {
        clipboard.setText(AnnotatedString(text))
        copied = true
    }
}

@Composable
fun CopyButton(text: String, lang: String? = null) {
    val tokens = Kampr.tokens
    val copying = rememberCopying(text)
    Row(
        Modifier
            .touchable(LANDSCAPE_TOUCH)
            .action(if (copying.copied) "Copied" else "Copy the ${lang ?: "code"} block", copying.copy),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(5.dp),
    ) {
        IconGlyph(ConversationIcons.copy, 11.dp, copying.tint)
        KText(if (copying.copied) "Copied" else "Copy", tokens.type.micro, copying.tint)
    }
}

// Prose has no bar to hang a caption in and no width to spare for one, so the same control
// paints its glyph alone. Nothing it paints is a word, which is also what keeps it out of a
// selection dragged across the quote it sits beside.
@Composable
fun CopyGlyph(text: String, what: String, modifier: Modifier = Modifier) {
    val copying = rememberCopying(text)
    GlyphTarget(
        ConversationIcons.copy,
        if (copying.copied) "Copied" else "Copy the $what",
        copying.tint,
        copying.copy,
        modifier,
        target = LANDSCAPE_TOUCH,
        glyph = 12.dp,
    )
}
