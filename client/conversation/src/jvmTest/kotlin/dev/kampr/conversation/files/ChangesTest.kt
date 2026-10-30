package dev.kampr.conversation.files

import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.test.waitUntilAtLeastOneExists
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.net.diffAttachmentId
import dev.kampr.shared.net.fileAttachmentId
import dev.kampr.shared.net.stagedAttachmentId
import dev.kampr.shared.net.unstagedAttachmentId
import dev.kampr.shared.ui.filePlace
import kotlin.test.Test
import kotlin.test.assertEquals

private const val PANE = "01JNODE/w3:p2"
private const val NOTES = "/home/u/demo/notes.kt"

private fun patch(old: String, new: String) = """diff --git a/notes.kt b/notes.kt
--- a/notes.kt
+++ b/notes.kt
@@ -1,1 +1,1 @@
-$old
+$new
"""

private val STAGED = patch("one", "staged two")
private val UNSTAGED = patch("staged two", "worktree three")
private val ALL = """diff --git a/notes.kt b/notes.kt
--- a/notes.kt
+++ b/notes.kt
@@ -1,2 +1,2 @@
-one
-also
+worktree three
+also too
"""

@OptIn(ExperimentalTestApi::class)
class ChangesTest {
    @Test
    fun whichViewsAreOffered() {
        val table = listOf(
            Triple(ALL, STAGED, UNSTAGED) to listOf(Side.All, Side.Staged, Side.Unstaged),
            Triple(STAGED, STAGED, "") to listOf(Side.Staged),
            Triple(UNSTAGED, null, UNSTAGED) to listOf(Side.Unstaged),
            Triple(ALL, null, null) to listOf(Side.All),
            Triple("", "", " \n") to emptyList(),
        )
        for ((given, want) in table) {
            assertEquals(want, changeViews(given.first, given.second, given.third).map { it.side }, "$given")
        }
    }

    @Test
    fun changesInTheIndexAndBesideItAreAThreeWaySwitch() = runComposeUiTest {
        val node = FileNode(
            mapOf(
                fileAttachmentId(NOTES) to text("worktree three\n"),
                diffAttachmentId(NOTES) to text(ALL),
                stagedAttachmentId(NOTES) to text(STAGED),
                unstagedAttachmentId(NOTES) to text(UNSTAGED),
            ),
        )
        setContent { Stage(node, Stack(filePlace(PANE, NOTES))) }
        waitUntilAtLeastOneExists(hasContentDescription("Changes +2 −2 tab"), 5_000)
        onNodeWithContentDescription("Changes +2 −2 tab").performClick()
        waitUntilAtLeastOneExists(hasText("+also too"), 5_000)
        onNodeWithContentDescription("Staged +1 −1 changes").performClick()
        waitUntilAtLeastOneExists(hasText("+staged two"), 5_000)
        onNodeWithContentDescription("Unstaged +1 −1 changes").performClick()
        waitUntilAtLeastOneExists(hasText("+worktree three"), 5_000)
        onNodeWithText("-staged two").assertExists()
    }

    @Test
    fun onlyStagedChangesSayAllOfItIsStaged() = runComposeUiTest {
        val node = FileNode(
            mapOf(
                fileAttachmentId(NOTES) to text("staged two\n"),
                diffAttachmentId(NOTES) to text(STAGED),
                stagedAttachmentId(NOTES) to text(STAGED),
                unstagedAttachmentId(NOTES) to AttachmentBytes.Failed("no such attachment", missing = true),
            ),
        )
        setContent { Stage(node, Stack(filePlace(PANE, NOTES))) }
        waitUntilAtLeastOneExists(hasContentDescription("Changes +1 −1 tab"), 5_000)
        onNodeWithContentDescription("Changes +1 −1 tab").performClick()
        waitUntilAtLeastOneExists(hasText("All of it is staged."), 5_000)
    }
}
