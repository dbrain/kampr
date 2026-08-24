package dev.kampr.conversation

import dev.kampr.conversation.md.Align
import dev.kampr.conversation.md.MdBlock
import dev.kampr.conversation.md.parseMarkdown
import dev.kampr.conversation.syntax.Token
import dev.kampr.conversation.syntax.langSpec
import dev.kampr.conversation.syntax.scan
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class MarkdownTest {
    @Test
    fun tableBecomesATableNotAParagraph() {
        val blocks = parseMarkdown(
            """
            | Key | send_keys | we send |
            |---|:-:|---:|
            | PageUp | rejected | ESC[5~ |
            | Home | rejected | ESC[H |
            """.trimIndent()
        )
        val table = blocks.single() as MdBlock.Table
        assertEquals(listOf("Key", "send_keys", "we send"), table.header)
        assertEquals(listOf(Align.Start, Align.Center, Align.End), table.aligns)
        assertEquals(2, table.rows.size)
        assertEquals(listOf("Home", "rejected", "ESC[H"), table.rows[1])
    }

    @Test
    fun tableRowsAreSquaredOffToTheHeaderWidth() {
        val table = parseMarkdown("| a | b | c |\n|---|---|---|\n| 1 |\n").single() as MdBlock.Table
        assertEquals(listOf("1", "", ""), table.rows.single())
    }

    // Agent output describes shell pipelines, so a pipe inside a code span is ordinary and must
    // not split the cell it lives in.
    @Test
    fun pipesInsideCodeSpansAndEscapesDoNotSplitCells() {
        val table = parseMarkdown("| cmd | note |\n|---|---|\n| `ls \\| wc` | a \\| b |\n").single() as MdBlock.Table
        assertEquals(listOf("`ls | wc`", "a | b"), table.rows.single())
    }

    @Test
    fun fencedCodeKeepsItsLanguageAndBody() {
        val fence = parseMarkdown("```rust\nlet x = 1;\n```\n").single() as MdBlock.Fence
        assertEquals("rust", fence.lang)
        assertEquals("let x = 1;", fence.code)
    }

    @Test
    fun aTableInsideAFenceStaysCode() {
        val blocks = parseMarkdown("```\n| a | b |\n|---|---|\n```\n")
        assertTrue(blocks.single() is MdBlock.Fence)
    }

    @Test
    fun nestedListsQuotesAndHeadingsParse() {
        val blocks = parseMarkdown(
            """
            ## What changed

            1. `pane.read recent` is capped, so:
               - paging back is impossible
               - the node accumulates
            2. No event fires on resize.

            > A short read can also be truncated.
            """.trimIndent()
        )
        assertEquals(2, (blocks[0] as MdBlock.Heading).level)
        val list = blocks[1] as MdBlock.Bullets
        assertTrue(list.ordered)
        assertEquals(2, list.items.size)
        val nested = list.items[0].blocks.filterIsInstance<MdBlock.Bullets>().single()
        assertEquals(2, nested.items.size)
        assertTrue(blocks[2] is MdBlock.Quote)
    }

    @Test
    fun inlineMarkupIsSpannedNotLeftAsSyntax() {
        val styles = testInlineStyles()
        val text = inlineText("**bold** *em* ~~gone~~ `code` [doc](https://kampr.dev)", styles)
        assertEquals("bold em gone code doc", text.text)
    }

    // Markdown is agent output, which may quote fetched web content: a scheme that can execute
    // must never become a live link.
    @Test
    fun onlyNavigableSchemesBecomeLinks() {
        val styles = testInlineStyles()
        val safe = inlineText("[go](https://kampr.dev)", styles)
        val unsafe = inlineText("[go](javascript:alert(1))", styles)
        assertEquals(1, linkCount(safe))
        assertEquals(0, linkCount(unsafe))
        assertEquals("go", unsafe.text)
    }

    // Kampr cannot carry image bytes, so a picture in agent prose is named rather than shown. It
    // used to render as a bare `!` beside link-styled alt text, and as nothing but `!` when the
    // alt was empty — a broken-looking artefact where a screenshot had been.
    @Test
    fun anImageIsNamedRatherThanRenderedAsABangAndALink() {
        val styles = testInlineStyles()
        val named = inlineText("before ![Screenshot](/tmp/shot.png) after", styles)
        assertEquals("before [image · Screenshot] after", named.text)
        assertEquals(0, linkCount(named))
        assertEquals("[image]", inlineText("![](https://kampr.dev/a.png)", styles).text)
    }

    // The node names a transcript image with this exact marker, in an ordinary md block, because
    // an older client drops a `b` value it does not know. The renderer must leave it alone.
    @Test
    fun theNodesImageMarkerSurvivesTheInlineParser() {
        val styles = testInlineStyles()
        assertEquals("[image · png]", inlineText("[image · png]", styles).text)
    }

    @Test
    fun unterminatedMarkupIsRenderedLiterally() {
        val styles = testInlineStyles()
        assertEquals("a *b c", inlineText("a *b c", styles).text)
        assertEquals("2 * 3 * 4", inlineText("2 \\* 3 \\* 4", styles).text)
    }

    @Test
    fun highlighterFindsStringsKeywordsAndComments() {
        val code = "let grid = Grid::new(74, 30); // fits\n"
        val spans = scan(code, langSpec("rust"))
        assertTrue(spans.any { it.token == Token.Keyword && code.substring(it.start, it.end) == "let" })
        assertTrue(spans.any { it.token == Token.Call && code.substring(it.start, it.end) == "new" })
        assertTrue(spans.any { it.token == Token.Comment && code.substring(it.start, it.end) == "// fits" })
        assertTrue(spans.any { it.token == Token.Number && code.substring(it.start, it.end) == "74" })
    }

    @Test
    fun shellHashIsACommentButRustHashIsAnAttribute() {
        assertTrue(scan("# note\nls", langSpec("bash")).any { it.token == Token.Comment })
        assertTrue(scan("#[derive(Debug)]\nstruct A;", langSpec("rust")).any { it.token == Token.Meta })
    }
}
