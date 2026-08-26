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

fun linkUrls(text: AnnotatedString): List<String> =
    text.getLinkAnnotations(0, text.length).mapNotNull { (it.item as? LinkAnnotation.Url)?.url }

fun linkedText(text: AnnotatedString): List<String> =
    text.getLinkAnnotations(0, text.length).map { text.text.substring(it.start, it.end) }
