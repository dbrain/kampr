package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicText
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.unit.dp
import dev.kampr.conversation.md.markUrls
import dev.kampr.conversation.syntax.langSpec
import dev.kampr.conversation.syntax.scan
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.glyphFallback
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.Surface
import dev.kampr.shared.ui.named

@Composable
fun CodeCard(lang: String?, code: String, query: String, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val palette = rememberConversationPalette()
    val body = remember(code, lang, palette) { highlight(code, lang, palette) }
    val marked = remember(body, query, palette) { body.markUrls(CODE_LINK).markMatches(query, palette.match) }

    // The card is the width of its longest line, capped by the frame — a one-line fence drawn a
    // metre wide is a box with nothing in it, and a wide one keeps its own scroller either way.
    Surface(modifier.fitContent(), background = palette.codeGround, radius = tokens.radii.md) {
        Column {
            Row(
                Modifier.fillMaxWidth().background(palette.codeBar).padding(horizontal = 11.dp, vertical = 7.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                KText(lang ?: "text", tokens.type.meta, tokens.color.mute)
                // A caption is chrome: dragging across the card must copy the code, not the word
                // sitting above it offering to copy the code.
                DisableSelection { CopyButton(code, lang) }
            }
            Box(Modifier.fillMaxWidth().height(1.dp).background(palette.rule))
            val style = tokens.type.caption.copy(fontFamily = tokens.fonts.mono, color = palette.plain)
            BasicText(
                text = marked.glyphFallback(style),
                modifier = Modifier
                    .fillMaxWidth()
                    .horizontalScroll(rememberScrollState())
                    .padding(horizontal = 11.dp, vertical = 10.dp),
                style = style,
                softWrap = false,
            )
        }
    }
}

fun highlight(code: String, lang: String?, palette: ConversationPalette): AnnotatedString {
    val spec = langSpec(lang)
    val spans = scan(code, spec)
    return buildAnnotatedString {
        append(code)
        for (span in spans) addStyle(SpanStyle(color = palette.of(span.token)), span.start, span.end)
    }
}
