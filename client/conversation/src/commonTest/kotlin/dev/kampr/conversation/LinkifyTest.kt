package dev.kampr.conversation

import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import dev.kampr.conversation.md.markUrls
import kotlin.test.Test
import kotlin.test.assertEquals

// An agent writes a URL the way a person does — in running prose, with no brackets round it — and
// a transcript that only linkifies `[label](target)` leaves the reader retyping it off a phone.
class LinkifyTest {
    private val styles = testInlineStyles()

    private fun urls(source: String) = linkUrls(inlineText(source, styles))

    @Test
    fun aUrlWrittenBareInProseIsALinkWithoutBeingWrittenAsOne() {
        val text = inlineText("The node is at https://kampr.dev/docs and stays there.", styles)
        assertEquals(listOf("https://kampr.dev/docs"), linkUrls(text))
        assertEquals(listOf("https://kampr.dev/docs"), linkedText(text))
        assertEquals("The node is at https://kampr.dev/docs and stays there.", text.text)
    }

    @Test
    fun theSentencesOwnPunctuationIsNotPartOfTheUrl() {
        assertEquals(listOf("https://example.com/a"), urls("See https://example.com/a."))
        assertEquals(listOf("https://example.com/a"), urls("See https://example.com/a, then stop"))
        assertEquals(listOf("https://example.com/a"), urls("Is it https://example.com/a?"))
        assertEquals(listOf("https://example.com/a"), urls("Read https://example.com/a!"))
    }

    // A URL is routinely written inside brackets, and a URL routinely contains brackets of its
    // own. Only the ones it did not open belong to the sentence.
    @Test
    fun onlyTheClosingParenthesisTheUrlDidNotOpenBelongsToTheSentence() {
        assertEquals(listOf("https://example.com/a"), urls("(see https://example.com/a)"))
        assertEquals(
            listOf("https://en.wikipedia.org/wiki/Terminal_(macOS)"),
            urls("see https://en.wikipedia.org/wiki/Terminal_(macOS)"),
        )
        assertEquals(
            listOf("https://en.wikipedia.org/wiki/Terminal_(macOS)"),
            urls("(see https://en.wikipedia.org/wiki/Terminal_(macOS))"),
        )
    }

    @Test
    fun aUrlInsideACodeSpanIsTextTheReaderWasShownAndNotALink() {
        val text = inlineText("run `curl https://example.com/a` first", styles)
        assertEquals(emptyList(), linkUrls(text))
        assertEquals("run curl https://example.com/a first", text.text)
    }

    // Only schemes that can do nothing but navigate, exactly as `[label](target)` is already
    // filtered: agent prose may be quoting fetched web content.
    @Test
    fun aSchemeThatCanExecuteIsNotLinkifiedWhenItIsWrittenBare() {
        assertEquals(emptyList(), urls("javascript:alert(1) is not a link"))
        assertEquals(emptyList(), urls("data:text/html,<b>x</b> is not a link"))
        assertEquals(emptyList(), urls("file:///etc/passwd is not a link"))
        assertEquals(listOf("mailto:agent@kampr.dev"), urls("write to mailto:agent@kampr.dev"))
    }

    @Test
    fun aSchemeInTheMiddleOfAWordIsNotAUrl() {
        assertEquals(emptyList(), urls("xhttps://example.com"))
        assertEquals(emptyList(), urls("shttp://example.com"))
    }

    // The label of a written link is already the link; scanning it again would stack a second
    // annotation over the same characters and point it somewhere else.
    @Test
    fun aWrittenLinkWhoseLabelIsAUrlStaysOneLinkToItsOwnTarget() {
        val text = inlineText("[https://shown.example](https://target.example/go)", styles)
        assertEquals(listOf("https://target.example/go"), linkUrls(text))
    }

    @Test
    fun anAutolinkIsNotLinkifiedTwice() {
        assertEquals(listOf("https://example.com/a"), urls("<https://example.com/a>"))
    }

    @Test
    fun emphasisAroundABareUrlStillLeavesItTappable() {
        assertEquals(listOf("https://example.com/a"), urls("**https://example.com/a**"))
        assertEquals(listOf("https://example.com/a"), urls("*https://example.com/a*"))
    }

    @Test
    fun everyUrlInAParagraphIsFound() {
        assertEquals(
            listOf("https://a.example", "http://b.example/x"),
            urls("first https://a.example then http://b.example/x done"),
        )
    }

    // Tool output and code are rendered as plain text rather than markdown, and a URL a reader can
    // see there — a docs link in an error, the address in a `curl` — is worth the same tap.
    @Test
    fun plainTextAlsoOffersTheUrlsAReaderCanSee() {
        val marked = AnnotatedString("error: see https://kampr.dev/docs#e17 for the cause")
            .markUrls(SpanStyle())
        assertEquals(listOf("https://kampr.dev/docs#e17"), linkUrls(marked))
        assertEquals("error: see https://kampr.dev/docs#e17 for the cause", marked.text)
    }

    @Test
    fun plainTextWithNoUrlIsHandedBackUntouched() {
        val plain = AnnotatedString("nothing to see")
        assertEquals(plain, plain.markUrls(SpanStyle()))
    }
}
