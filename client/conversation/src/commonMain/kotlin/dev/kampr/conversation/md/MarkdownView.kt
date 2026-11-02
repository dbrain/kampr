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
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.kampr.conversation.CodeCard
import dev.kampr.conversation.CopyGlyph
import dev.kampr.conversation.markMatches
import dev.kampr.conversation.rememberConversationPalette
import dev.kampr.conversation.rememberInlineStyles
import dev.kampr.shared.model.mentionedDirs
import dev.kampr.shared.model.nearer
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.glyphFallback

@Composable
fun Markdown(source: String, query: String, modifier: Modifier = Modifier, breaks: Breaks = Breaks.Soft, near: List<String> = emptyList()) {
    val blocks = remember(source, breaks) { parseMarkdown(source, breaks) }
    MarkdownBlocks(blocks, query, modifier, near)
}

// `near` is the directories the reply named before these blocks, nearest first, so a bare name
// in one of them can be looked for where the reply said it was.
@Composable
fun MarkdownBlocks(blocks: List<MdBlock>, query: String, modifier: Modifier = Modifier, near: List<String> = emptyList()) {
    val nears = remember(blocks, near) { blocks.runningFold(near) { seen, block -> nearer(block.mentioned(), seen) } }
    Column(modifier, verticalArrangement = Arrangement.spacedBy(9.dp)) {
        for ((index, block) in blocks.withIndex()) MarkdownBlock(block, query, nears[index])
    }
}

private fun MdBlock.mentioned(): List<String> = when (this) {
    is MdBlock.Heading -> mentionedDirs(text)
    is MdBlock.Paragraph -> mentionedDirs(text)
    is MdBlock.Fence -> mentionedDirs(code)
    is MdBlock.Quote -> blocks.flatMap { it.mentioned() }
    is MdBlock.Table -> (header + rows.flatten()).flatMap(::mentionedDirs)
    is MdBlock.Bullets -> items.flatMap { item -> item.blocks.flatMap { it.mentioned() } }
    MdBlock.Rule -> emptyList()
}

@Composable
private fun MarkdownBlock(block: MdBlock, query: String, near: List<String>) {
    val tokens = Kampr.tokens
    val palette = rememberConversationPalette()
    val inline = rememberInlineStyles(palette)
    when (block) {
        is MdBlock.Paragraph -> {
            val text = remember(block, inline, query, palette, near) {
                inlineMarkdown(block.text, inline, near).markMatches(query, palette.match)
            }
            val style = tokens.type.body.copy(color = tokens.color.text)
            BasicText(text.glyphFallback(style), style = style)
        }

        is MdBlock.Heading -> {
            val text = remember(block, inline, query, palette, near) {
                inlineMarkdown(block.text, inline, near).markMatches(query, palette.match)
            }
            val style = headingStyle(block.level, tokens.type.body.copy(color = tokens.color.text))
            BasicText(text.glyphFallback(style), Modifier.padding(top = 4.dp), style = style)
        }

        is MdBlock.Fence -> CodeCard(block.lang, block.code, query, near = near)

        is MdBlock.Table -> MarkdownTable(block, query, near = near)

        MdBlock.Rule -> Box(Modifier.fillMaxWidth().height(1.dp).background(palette.rule))

        is MdBlock.Quote -> Row(
            Modifier.fillMaxWidth().quoteRule(tokens.color.accent),
            horizontalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            MarkdownBlocks(block.blocks, query, Modifier.weight(1f).padding(start = 12.dp), near)
            // A caption is chrome: dragging across the quote must copy what was quoted, not the
            // control offering to copy it.
            DisableSelection { CopyGlyph(block.text, "quote") }
        }

        is MdBlock.Bullets -> Column(
            Modifier.fillMaxWidth(),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            val nears = remember(block, near) { block.items.runningFold(near) { seen, item -> nearer(item.blocks.flatMap { it.mentioned() }, seen) } }
            for ((index, item) in block.items.withIndex()) {
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    val marker = tokens.type.body.copy(color = tokens.color.dim)
                    BasicText(
                        AnnotatedString(item.marker).glyphFallback(marker),
                        Modifier.width(if (block.ordered) 21.dp else 13.dp),
                        style = marker,
                    )
                    MarkdownBlocks(item.blocks, query, Modifier.weight(1f), nears[index])
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
