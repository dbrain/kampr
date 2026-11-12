package dev.kampr.conversation.files

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTextReplacement
import androidx.compose.ui.test.v2.runComposeUiTest
import androidx.compose.ui.test.waitUntilExactlyOneExists
import androidx.compose.ui.unit.dp
import dev.kampr.conversation.tokensFor
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.net.lsAttachmentId
import dev.kampr.shared.net.namesAttachmentId
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.FilesPlace
import dev.kampr.shared.ui.LocalPaneIo
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val NODE = "01JNODE"
private const val ANCHOR = "01JNODE/w3:p2"
private const val DIR = "/home/u/dev/kampr"

private val LISTING = text(
    """{"path":"$DIR","parent":"/home/u/dev","home":"/home/u","entries":[
  {"name":"src","kind":"dir","mtime":1730000000},
  {"name":"lib.rs","kind":"file","bytes":10,"mtime":1730000000}],"truncated":false}""",
)

private fun names(truncated: Boolean, vararg hits: Pair<String, Boolean>) = text(
    """{"matches":[${hits.joinToString(",") { (path, dir) -> """{"path":"$path","dir":$dir}""" }}],"truncated":$truncated,"indexed":9,"cut":false}""",
)

private val LIB = names(
    true,
    "$DIR/lib.rs" to false,
    "$DIR/src/library" to true,
    "$DIR/src/parse/lib.rs" to false,
)

@OptIn(ExperimentalTestApi::class)
class DirSearchTest {
    private fun ComposeUiTest.show(io: FileNode, nav: Stack) {
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                LocalPaneIo provides io,
            ) {
                Box(Modifier.size(411.dp, 914.dp)) { DirView(FilesPlace.Dir(NODE, ANCHOR, DIR), nav) }
            }
        }
        waitUntilExactlyOneExists(hasContentDescription("folder src", substring = true))
    }

    private fun search(query: String) = namesAttachmentId(DIR, query)

    @Test
    fun aSearchListsWhatIsBelowTheFolderUnderItsOwnMatchesAndOpensEach() = runComposeUiTest {
        val io = FileNode(mapOf(lsAttachmentId(DIR) to LISTING, search("lib") to LIB))
        val nav = Stack(FilesPlace.Dir(NODE, ANCHOR, DIR))
        show(io, nav)

        mainClock.autoAdvance = false
        onNodeWithContentDescription("Search this folder and below").performTextInput("li")
        mainClock.advanceTimeBy(100)
        onNodeWithContentDescription("Search this folder and below").performTextInput("b")
        mainClock.autoAdvance = true

        waitUntilExactlyOneExists(hasContentDescription("folder library, in src"))
        onNodeWithContentDescription("lib.rs, in src/parse").assertExists()
        waitUntilExactlyOneExists(hasText("Below kampr"))
        waitUntilExactlyOneExists(hasText("Showing the first 3"))
        assertEquals(listOf(lsAttachmentId(DIR), search("lib")), io.asked.map { it.second }, "one search once the typing settles")

        onNodeWithContentDescription("folder library, in src").performClick()
        onNodeWithContentDescription("lib.rs, in src/parse").performClick()
        assertEquals(
            listOf(FilesPlace.Dir(NODE, ANCHOR, "$DIR/src/library"), FilesPlace.File(NODE, ANCHOR, "$DIR/src/parse/lib.rs")),
            nav.places.drop(1),
        )
    }

    @Test
    fun aDirectChildAlreadyListedIsNotListedAgainBelow() = runComposeUiTest {
        val io = FileNode(mapOf(lsAttachmentId(DIR) to LISTING, search("lib") to LIB))
        show(io, Stack(FilesPlace.Dir(NODE, ANCHOR, DIR)))
        onNodeWithContentDescription("Search this folder and below").performTextInput("lib")
        waitUntilExactlyOneExists(hasContentDescription("folder library, in src"))
        onNodeWithContentDescription("lib.rs, 10 B", substring = true).assertExists()
        assertTrue(onAllNodesWithContentDescription("lib.rs").fetchSemanticsNodes().isEmpty(), "the folder's own lib.rs is not listed again below it")
    }

    @Test
    fun aSingleCharacterFiltersTheFolderAndAsksTheNodeNothing() = runComposeUiTest {
        val io = FileNode(mapOf(lsAttachmentId(DIR) to LISTING))
        show(io, Stack(FilesPlace.Dir(NODE, ANCHOR, DIR)))
        onNodeWithContentDescription("Search this folder and below").performTextInput("s")
        waitForIdle()
        mainClock.advanceTimeBy(1000)
        waitForIdle()
        assertEquals(listOf(lsAttachmentId(DIR)), io.asked.map { it.second })
        onNodeWithContentDescription("folder src", substring = true).assertExists()
    }

    @Test
    fun anEmptyAnswerAndARefusalEachSaySo() = runComposeUiTest {
        val io = FileNode(
            mapOf(
                lsAttachmentId(DIR) to LISTING,
                search("zz") to names(false),
                search("old") to AttachmentBytes.Failed("no such attachment", missing = true),
            ),
        )
        show(io, Stack(FilesPlace.Dir(NODE, ANCHOR, DIR)))
        val field = onNodeWithContentDescription("Search this folder and below")
        field.performTextInput("zz")
        waitUntilExactlyOneExists(hasText("Nothing below matches"))
        field.performTextReplacement("old")
        waitUntilExactlyOneExists(hasText(CANNOT_SEARCH_BELOW))
    }
}
