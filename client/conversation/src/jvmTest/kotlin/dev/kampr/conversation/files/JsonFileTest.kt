package dev.kampr.conversation.files

import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.v2.runComposeUiTest
import androidx.compose.ui.test.waitUntilAtLeastOneExists
import dev.kampr.shared.net.fileAttachmentId
import dev.kampr.shared.ui.filePlace
import kotlin.test.Test

private const val PANE = "01JNODE/w3:p2"

private val MINIFIED = """{"name":"kampr","deep":{"flag":true},"tags":[${(1..40).joinToString(",") { "\"tag-$it\"" }}]}"""

@OptIn(ExperimentalTestApi::class)
class JsonFileTest {
    @Test
    fun aMinifiedJsonOpensFormattedAndItsSourceIsAPressAway() = runComposeUiTest {
        val path = "/home/u/demo/data.json"
        setContent { Stage(FileNode(mapOf(fileAttachmentId(path) to text(MINIFIED))), Stack(filePlace(PANE, path))) }
        waitUntilAtLeastOneExists(hasText("    \"flag\": true"), 5_000)

        onNodeWithContentDescription("Source", substring = true).performClick()
        waitUntilAtLeastOneExists(hasText("\"deep\":{\"flag\":true}", substring = true), 5_000)
    }

    @Test
    fun aJsonAlreadyLaidOutOpensOnItsSource() = runComposeUiTest {
        val path = "/home/u/demo/pretty.json"
        val pretty = "{\n    \"a\": 1,\n    \"b\": 2\n}\n"
        setContent { Stage(FileNode(mapOf(fileAttachmentId(path) to text(pretty))), Stack(filePlace(PANE, path))) }
        waitUntilAtLeastOneExists(hasText("    \"a\": 1,"), 5_000)

        onNodeWithContentDescription("Formatted", substring = true).performClick()
        waitUntilAtLeastOneExists(hasText("  \"a\": 1,"), 5_000)
    }

    @Test
    fun jsonLinesAreNeverReformatted() = runComposeUiTest {
        val path = "/home/u/demo/events.jsonl"
        setContent { Stage(FileNode(mapOf(fileAttachmentId(path) to text("$MINIFIED\n$MINIFIED\n"))), Stack(filePlace(PANE, path))) }
        waitUntilAtLeastOneExists(hasText("\"deep\":{\"flag\":true}", substring = true), 5_000)
        waitForIdle()
        check(onAllNodes(hasContentDescription("Formatted", substring = true)).fetchSemanticsNodes().isEmpty())
    }
}
