package dev.kampr.conversation

import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import dev.kampr.conversation.md.InlineStyles
import dev.kampr.conversation.md.inlineMarkdown

fun testInlineStyles() = InlineStyles(SpanStyle(), SpanStyle())

fun inlineText(source: String, styles: InlineStyles): AnnotatedString = inlineMarkdown(source, styles)

fun linkCount(text: AnnotatedString): Int =
    text.getLinkAnnotations(0, text.length).count { it.item is LinkAnnotation.Url }
