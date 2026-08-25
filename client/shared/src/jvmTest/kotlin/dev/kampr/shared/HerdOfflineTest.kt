package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.Herd
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.on
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.HerdLandscape
import dev.kampr.shared.ui.HerdPortrait
import dev.kampr.shared.ui.HerdSidebar
import dev.kampr.shared.ui.connectionWord
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import org.jetbrains.skia.Bitmap
import java.io.File
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private fun tokens() = themeOf("soft").on(Ground.Dark).let { spec ->
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    KamprTokens(spec, fonts, typography(fonts, spec.label, TypeScale.Phone))
}

@Composable
private fun Themed(content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalTokens provides tokens(), content = content)
}

private val NODE = NodeInfo(id = "01JA", name = "comingclean", kind = "local", online = true)

private fun pane(id: String, status: String) = PaneInfo(
    id = "01JA/$id",
    nodeId = "01JA",
    workspace = "kampr",
    cwd = "/home/dbrain/dev/kampr",
    agent = "claude",
    agentStatus = status,
    cols = 74,
    rows = 30,
)

// No pane is Done: the healthy colour has to be the pill's or the pixel probe below proves nothing.
private val REMEMBERED = Herd(
    nodes = listOf(NODE),
    panes = listOf(pane("w1:p1", "working"), pane("w2:p1", "idle")),
    stale = true,
    known = true,
)

private val LIVE = ConnectionStatus.Live("full")
private val OFFLINE = ConnectionStatus.Offline("no route to host", 5_000)
private val REFUSED = ConnectionStatus.Refused("This node does not know this device.")

private fun layouts(herd: Herd, connection: ConnectionStatus): List<Pair<String, @Composable () -> Unit>> =
    listOf(
        "portrait" to {
            Box(Modifier.size(420.dp, 900.dp)) {
                HerdPortrait(herd, connection, 0.0, 12.0, emptyList(), {}, null)
            }
        },
        "landscape" to {
            Box(Modifier.size(900.dp, 420.dp)) {
                HerdLandscape(herd, connection, 0.0, 12.0, emptyList(), {}, null)
            }
        },
        "sidebar" to {
            Box(Modifier.size(300.dp, 900.dp)) {
                HerdSidebar(herd, connection, 0.0, 12.0, emptyList(), null, "this device", "full access", {}, {})
            }
        },
    )

// A device with no network drew the title, a green dot on "0 nodes" and a void, which is exactly
// what a healthy node with nothing running on it draws. The client's own spelling of #233.
@OptIn(ExperimentalTestApi::class)
class HerdOfflineTest {
    @Test
    fun aHerdWithNoNetworkDoesNotReadAsAHerdWithNoMachines() {
        for ((name, layout) in layouts(Herd(), OFFLINE)) runComposeUiTest {
            setContent { Themed { layout() } }
            waitForIdle()
            assertTrue(
                onAllNodesWithContentDescription("Reconnecting", substring = true).fetchSemanticsNodes().isNotEmpty(),
                "the $name layout said nothing at all about a device that had lost the node",
            )
            assertTrue(
                onAllNodesWithContentDescription("No panes yet", substring = true).fetchSemanticsNodes().isEmpty(),
                "the $name layout told a device with no network that its herd was simply empty",
            )
        }
    }

    // Four different pieces of news, and the operator can act on only some of them.
    @Test
    fun everyHerdLayoutSaysWhyItsBodyIsEmpty() {
        // The headline is the pill's word too, so the body is pinned by the sentence under it.
        val states = listOf(
            Triple(OFFLINE, "Reconnecting", "lost the node"),
            Triple(REFUSED, "Not paired with this node", "Pair it again from Settings"),
            Triple(ConnectionStatus.Connecting, "Connecting", "Reaching the node"),
            Triple(ConnectionStatus.Idle, "Not connected", "has not reached a node yet"),
            Triple(LIVE, "No panes yet", "nothing running on it"),
        )
        for ((connection, headline, detail) in states) {
            for ((name, layout) in layouts(Herd(known = true), connection)) runComposeUiTest {
                setContent { Themed { layout() } }
                waitForIdle()
                for (said in listOf(headline, detail)) {
                    assertTrue(
                        onAllNodesWithContentDescription(said, substring = true).fetchSemanticsNodes().isNotEmpty(),
                        "the $name layout rendered an empty body without saying \"$said\"",
                    )
                }
                for ((_, _, other) in states) {
                    if (other == detail) continue
                    assertTrue(
                        onAllNodesWithContentDescription(other, substring = true).fetchSemanticsNodes().isEmpty(),
                        "the $name layout said \"$other\" about a socket that was $connection",
                    )
                }
            }
        }
    }

    // A list that survived the socket is a memory, not a status, and the pane surfaces already
    // spell that fact "Stale".
    @Test
    fun aHerdKeptOnScreenAfterTheSocketDroppedReadsAsAMemory() {
        for ((name, layout) in layouts(REMEMBERED, OFFLINE)) runComposeUiTest {
            setContent { Themed { layout() } }
            waitForIdle()
            assertTrue(
                onAllNodesWithContentDescription("Stale —", substring = true).fetchSemanticsNodes().isNotEmpty(),
                "the $name layout kept painting a dead herd as if it were the herd now",
            )
        }
        for ((name, layout) in layouts(REMEMBERED.copy(stale = false), LIVE)) runComposeUiTest {
            setContent { Themed { layout() } }
            waitForIdle()
            assertTrue(
                onAllNodesWithContentDescription("Stale —", substring = true).fetchSemanticsNodes().isEmpty(),
                "the $name layout called a live herd stale",
            )
        }
    }

    // The pill's label is the second place the lie was told, and the only place a screen reader
    // could hear it.
    @Test
    fun theMachinePillSpeaksTheSocketAndNotOnlyTheCount() {
        for ((name, layout) in layouts(Herd(), OFFLINE)) runComposeUiTest {
            setContent { Themed { layout() } }
            waitForIdle()
            onNodeWithContentDescription("Machines — 0 nodes online, Reconnecting").assertExists()
            assertTrue(
                onAllNodesWithContentDescription("Machines — 0 nodes online").fetchSemanticsNodes().isEmpty(),
                "the $name pill still announced a bare healthy count while the socket was down",
            )
        }
    }

    // Semantics cannot see a colour, and the colour was the whole of the report: a green dot on a
    // device that had never reached anything. Rendered, and counted.
    @Test
    fun theHerdPaintsNoHealthyMarkWhileTheSocketIsDown() {
        val healthy = tokensFor(themeOf("soft"), TypeScale.Phone, Ground.Dark).color.done.toArgb()
        fun healthyPixels(connection: ConnectionStatus, name: String): Int {
            val image = render(
                390.dp, 240.dp, themeOf("soft"), TypeScale.Phone, File("build/artboards/herd-$name.png"),
            ) {
                HerdPortrait(REMEMBERED.copy(stale = false), connection, 0.0, 12.0, emptyList(), {}, null)
            }
            val bitmap = Bitmap.makeFromImage(image)
            var hits = 0
            for (y in 0 until bitmap.height) {
                for (x in 0 until bitmap.width) if (bitmap.getColor(x, y) == healthy) hits++
            }
            return hits
        }
        assertTrue(
            healthyPixels(LIVE, "live") > 0,
            "a live socket painted nothing in the healthy colour, so this probe proves nothing",
        )
        assertEquals(
            0,
            healthyPixels(OFFLINE, "offline"),
            "the herd painted the healthy colour at a device that had lost the node",
        )
        assertEquals(
            0,
            healthyPixels(ConnectionStatus.Idle, "idle"),
            "the herd painted the healthy colour at a device that had never reached a node",
        )
    }

    // One vocabulary, shared with the detail pane. Offline and Refused are the pair that must
    // never collapse: one comes back on its own and one never will.
    @Test
    fun theSocketHasOneSetOfWordsAndRefusedIsNotOffline() {
        val words = listOf(OFFLINE, REFUSED, ConnectionStatus.Connecting, ConnectionStatus.Idle)
            .map { connectionWord(it) }
        assertTrue(words.all { !it.isNullOrBlank() }, "a socket state had no word of its own: $words")
        assertEquals(words.size, words.toSet().size, "two socket states share a word: $words")
        assertEquals(null, connectionWord(LIVE), "a live socket is not news, and every surface says its own thing")
    }
}
