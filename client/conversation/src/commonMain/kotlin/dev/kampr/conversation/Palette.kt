package dev.kampr.conversation

import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.compositeOver
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import dev.kampr.conversation.md.InlineStyles
import dev.kampr.conversation.syntax.Token
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.theme.KamprTokens

// Every colour here is a token read through KamprTokens, so a second theme moves the whole
// transcript with it — syntax highlighting included, which is where hardcoded palettes usually
// leak in.
@Immutable
class ConversationPalette(tokens: KamprTokens) {
    private val color = tokens.color

    val keyword: Color = color.accent
    val literal: Color = color.done
    val number: Color = color.accentHi
    val comment: Color = color.mute
    val punct: Color = color.dim
    val call: Color = color.text
    val meta: Color = color.working
    val plain: Color = color.text

    val codeGround: Color = color.surface2
    val codeBar: Color = color.surface
    val chipGround: Color = color.raise
    val rule: Color = color.line

    val added: Color = color.done
    val removed: Color = color.blocked
    val addedGround: Color = color.done.copy(alpha = 0.12f).compositeOver(color.surface2)
    val removedGround: Color = color.blocked.copy(alpha = 0.12f).compositeOver(color.surface2)
    val hunk: Color = color.accent
    val hunkGround: Color = color.accent.copy(alpha = 0.10f).compositeOver(color.surface2)

    val headerGround: Color = color.surface

    val match: Color = color.working.copy(alpha = 0.34f).compositeOver(color.bg)

    fun of(token: Token): Color = when (token) {
        Token.Keyword -> keyword
        Token.Text -> literal
        Token.Number -> number
        Token.Comment -> comment
        Token.Punct -> punct
        Token.Call -> call
        Token.Meta -> meta
        Token.Plain -> plain
    }
}

@Composable
fun rememberConversationPalette(): ConversationPalette {
    val tokens = Kampr.tokens
    return remember(tokens) { ConversationPalette(tokens) }
}

@Composable
fun rememberInlineStyles(palette: ConversationPalette): InlineStyles {
    val tokens = Kampr.tokens
    return remember(tokens, palette) {
        InlineStyles(
            code = SpanStyle(
                fontFamily = tokens.fonts.mono,
                fontSize = tokens.type.caption.fontSize,
                background = palette.chipGround,
                color = tokens.color.text,
            ),
            link = SpanStyle(
                color = tokens.color.accent,
                textDecoration = TextDecoration.Underline,
                fontWeight = FontWeight.W500,
            ),
        )
    }
}
