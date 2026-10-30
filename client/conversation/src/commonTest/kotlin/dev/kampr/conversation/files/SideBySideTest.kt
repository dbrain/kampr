package dev.kampr.conversation.files

import kotlin.test.Test
import kotlin.test.assertEquals

private val GIT_DIFF = """
    diff --git a/notes.md b/notes.md
    index 1111111..2222222 100644
    --- a/notes.md
    +++ b/notes.md
    @@ -3,5 +3,6 @@ heading
     kept
    -gone one
    -gone two
    -gone three
    +came one
    +came two
     still here
    +brand new
    \ No newline at end of file
""".trimIndent()

private fun SideRow.Pair.shape(): String = "${left?.number ?: "_"}:${left?.text ?: ""}|${right?.number ?: "_"}:${right?.text ?: ""}"

class SideBySideTest {
    @Test
    fun removedAndAddedRunsPairLineByLineWithEachSideNumberedFromTheHunk() {
        val rows = sideBySide(GIT_DIFF)
        assertEquals(SideRow.Hunk("@@ -3,5 +3,6 @@ heading"), rows.first(), "the header lines before the hunk leaked in")
        assertEquals(
            listOf(
                "3:kept|3:kept",
                "4:gone one|4:came one",
                "5:gone two|5:came two",
                "6:gone three|_:",
                "7:still here|6:still here",
                "_:|7:brand new",
            ),
            rows.drop(1).map { (it as SideRow.Pair).shape() },
        )
    }

    @Test
    fun aChangeOnOneSideOnlyLeavesTheOtherSideBlankRatherThanMisaligned() {
        val rows = sideBySide("@@ -1,2 +1,0 @@\n-a\n-b\n@@ -9 +7,2 @@\n+c\n+d\n")
        assertEquals(
            listOf("1:a|_:", "2:b|_:"),
            rows.subList(1, 3).map { (it as SideRow.Pair).shape() },
        )
        assertEquals(SideRow.Hunk("@@ -9 +7,2 @@"), rows[3])
        assertEquals(listOf("_:|7:c", "_:|8:d"), rows.drop(4).map { (it as SideRow.Pair).shape() })
    }

    @Test
    fun aDiffWithNoHunksHasNoRows() {
        assertEquals(emptyList(), sideBySide(""))
        assertEquals(emptyList(), sideBySide("diff --git a/x b/x\nnew file mode 100644\n"))
    }

    @Test
    fun changesAreCountedOnlyInsideHunks() {
        assertEquals(DiffCounts(added = 3, removed = 3), diffCounts(GIT_DIFF), "the ---/+++ header lines were counted")
    }
}
