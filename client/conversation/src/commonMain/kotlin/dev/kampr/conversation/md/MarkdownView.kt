package dev.kampr.conversation.md

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.text.BasicText
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.text.selection.DisableSelection
import dev.kampr.conversation.CodeCard
import dev.kampr.conversation.CopyGlyph
import dev.kampr.conversation.markMatches
import dev.kampr.conversation.rememberConversationPalette
import dev.kampr.conversation.rememberInlineStyles
import androidx.compose.ui.text.AnnotatedString
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.glyphFallback

@Composable
fun Markdown(source: String, query: String, modifier: Modifier = Modifier, breaks: Breaks = Breaks.Soft) {
    val blocks = remember(source, breaks) { parseMarkdown(source, breaks) }
    MarkdownBlocks(blocks, query, modifier)
}

@Composable
fun MarkdownBlocks(blocks: List<MdBlock>, query: String, modifier: Modifier = Modifier) {
    Column(modifier, verticalArrangement = Arrangement.spacedBy(9.dp)) {
        for (block in blocks) MarkdownBlock(block, query)
    }
}

@Composable
private fun MarkdownBlock(block: MdBlock, query: String) {
    val tokens = Kampr.tokens
    val palette = rememberConversationPalette()
    val inline = rememberInlineStyles(palette)
    when (block) {
        is MdBlock.Paragraph -> {
            val text = remember(block, inline, query, palette) {
                inlineMarkdown(block.text, inline).markMatches(query, palette.match)
            }
            val style = tokens.type.body.copy(color = tokens.color.text)
            BasicText(text.glyphFallback(style), style = style)
        }

        is MdBlock.Heading -> {
            val text = remember(block, inline, query, palette) {
                inlineMarkdown(block.text, inline).markMatches(query, palette.match)
            }
            val style = headingStyle(block.level, tokens.type.body.copy(color = tokens.color.text))
            BasicText(text.glyphFallback(style), Modifier.padding(top = 4.dp), style = style)
        }

        is MdBlock.Fence -> CodeCard(block.lang, block.code, query)

        is MdBlock.Table -> MarkdownTable(block, query)

        MdBlock.Rule -> Box(Modifier.fillMaxWidth().height(1.dp).background(palette.rule))

        is MdBlock.Quote -> Row(
            Modifier.fillMaxWidth().quoteRule(tokens.color.accent),
            horizontalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            MarkdownBlocks(block.blocks, query, Modifier.weight(1f).padding(start = 12.dp))
            // A caption is chrome: dragging across the quote must copy what was quoted, not the
            // control offering to copy it.
            DisableSelection { CopyGlyph(block.text, "quote") }
        }

        is MdBlock.Bullets -> Column(
            Modifier.fillMaxWidth(),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            for (item in block.items) {
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    val marker = tokens.type.body.copy(color = tokens.color.dim)
                    BasicText(
                        AnnotatedString(item.marker).glyphFallback(marker),
                        Modifier.width(if (block.ordered) 21.dp else 13.dp),
                        style = marker,
                    )
                    MarkdownBlocks(item.blocks, query, Modifier.weight(1f))
                }
            }
        }
    }
}

// Drawn rather than laid out: a quote can contain a table, and a table measures against the
// width it is given, which rules out asking the row for an intrinsic height.
private fun Modifier.quoteRule(color: Color): Modifier = drawBehind {
    drawRect(color, Offset.Zero, Size(2f * density, size.height))
}

private fun headingStyle(level: Int, body: TextStyle): TextStyle {
    val scale = when (level) {
        1 -> 1.44f
        2 -> 1.26f
        3 -> 1.13f
        else -> 1.04f
    }
    return body.copy(
        fontSize = body.fontSize * scale,
        lineHeight = body.lineHeight * scale,
        fontWeight = if (level <= 2) FontWeight.W800 else FontWeight.W700,
    )
}
