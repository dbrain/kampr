package dev.kampr.conversation

import dev.kampr.conversation.md.Breaks
import dev.kampr.conversation.md.MdBlock
import dev.kampr.conversation.md.parseMarkdown
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private fun text(source: String, breaks: Breaks): String =
    (parseMarkdown(source, breaks).first() as MdBlock.Paragraph).text

// CommonMark's soft break — a single newline inside a paragraph is a space — is right for an agent,
// which writes real markdown and wraps its own lines, and wrong for a person, who pressed Enter and
// meant it. The reported symptom was the operator's own message losing every line break it was
// typed with.
class HardBreakTest {
    @Test
    fun a_wrapped_line_from_an_agent_is_one_line_and_a_typed_one_is_two() {
        assertEquals("first line second line", text("first line\nsecond line\n", Breaks.Soft))
        assertEquals("first line\nsecond line", text("first line\nsecond line\n", Breaks.Hard))
    }

    // The fix must not be "render a person's words as plain text": people paste fences, bullets and
    // tables into a reply and every one of them has to go on being what it is.
    @Test
    fun everything_that_is_not_a_soft_break_renders_the_same_in_both_modes() {
        for (breaks in Breaks.entries) {
            val listed = parseMarkdown("what I want:\n- one\n- two\n", breaks)
            assertEquals(2, listed.size, "$breaks lost the boundary between prose and a list")
            assertEquals("what I want:", (listed[0] as MdBlock.Paragraph).text)
            assertEquals(2, (listed[1] as MdBlock.Bullets).items.size)

            val fence = parseMarkdown("```sh\nls\n\nwc\n```\n", breaks).single() as MdBlock.Fence
            assertEquals("sh", fence.lang)
            assertEquals("ls\n\nwc", fence.code, "$breaks reflowed the inside of a fence")

            val table = parseMarkdown("| a | b |\n|---|---|\n| 1 | 2 |\n", breaks).single() as MdBlock.Table
            assertEquals(listOf("a", "b"), table.header)
            assertEquals(listOf(listOf("1", "2")), table.rows)

            val quoted = parseMarkdown("> one\n> two\n", breaks).single() as MdBlock.Quote
            assertTrue(quoted.blocks.single() is MdBlock.Paragraph)
        }
    }

    // Two trailing spaces are CommonMark's own hard break, and they never worked here: the line was
    // trimmed before anything could read them and the join put a space back.
    @Test
    fun two_trailing_spaces_break_the_line_whoever_typed_them() {
        for (breaks in Breaks.entries) {
            assertEquals(
                "Roses\nare red",
                text("Roses  \nare red\n", breaks),
                "$breaks dropped a hard break the writer asked for outright",
            )
        }
    }

    // The break has to reach a paragraph wherever it is, not only one at the top level.
    @Test
    fun a_typed_break_survives_inside_a_quote_and_inside_a_list_item() {
        val quoted = parseMarkdown("> one\n> two\n", Breaks.Hard).single() as MdBlock.Quote
        assertEquals("one\ntwo", (quoted.blocks.single() as MdBlock.Paragraph).text)

        val listed = parseMarkdown("- one\n  two\n", Breaks.Hard).single() as MdBlock.Bullets
        assertEquals("one\ntwo", (listed.items.single().blocks.single() as MdBlock.Paragraph).text)
    }
}
