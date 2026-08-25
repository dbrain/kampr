package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
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
import dev.kampr.shared.wire.NodeInfo
import kotlin.test.Test
import kotlin.test.assertTrue

private fun tokens() = themeOf("soft").on(Ground.Dark).let { spec ->
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    KamprTokens(spec, fonts, typography(fonts, spec.label, TypeScale.Phone))
}

@Composable
private fun Themed(content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalTokens provides tokens(), content = content)
}

private val LOCAL = NodeInfo(id = "01JA", name = "comingclean", kind = "local", build = "0.1.2")
private val PEER = NodeInfo(id = "01JB", name = "haymaker", kind = "peer", herdrVersion = "0.9.1")
private val DOWN = NodeInfo(
    id = "01JC",
    name = "stopgap",
    kind = "peer",
    online = false,
    detail = "no route to host",
)

private const val LOCAL_ROW = "comingclean, this machine · online · kampr 0.1.2"
private const val PEER_ROW = "haymaker, peer · online · herdr 0.9.1"
private const val DOWN_ROW = "stopgap, peer · offline · no route to host"

private val HERD = Herd(nodes = listOf(LOCAL, PEER, DOWN), known = true)
private val ALONE = Herd(nodes = listOf(LOCAL), known = true)

// The pill on the front page counted nodes and did nothing else — a control with nothing behind
// it — and its plural was unconditional, so a herd of one read "1 nodes".
@OptIn(ExperimentalTestApi::class)
class HerdNodeListTest {
    @Test
    fun aHerdOfOneNodeReadsAsOneNodeRatherThanOneNodes() = runComposeUiTest {
        setContent { Themed { Box(Modifier.size(420.dp, 900.dp)) { HerdPortrait(ALONE, ConnectionStatus.Live("full"), 0.0, 12.0, emptyList(), {}, null) } } }
        waitForIdle()
        assertTrue(
            onAllNodesWithText("1 nodes", substring = true, useUnmergedTree = true).fetchSemanticsNodes().isEmpty(),
            "a herd of one machine still painted a plural",
        )
        assertTrue(
            onAllNodesWithText("1 node", useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty(),
            "the pill lost its count",
        )
    }

    @Test
    fun thePillOpensAListOfEveryMachineWithItsRealStatus() = runComposeUiTest {
        setContent { Themed { Box(Modifier.size(420.dp, 900.dp)) { HerdPortrait(HERD, ConnectionStatus.Live("full"), 0.0, 12.0, emptyList(), {}, null) } } }
        waitForIdle()
        for (row in listOf(LOCAL_ROW, PEER_ROW, DOWN_ROW)) {
            assertTrue(
                onAllNodesWithContentDescription(row).fetchSemanticsNodes().isEmpty(),
                "the machine list was open before anybody asked for it",
            )
        }
        onNodeWithContentDescription("nodes online", substring = true).performClick()
        waitForIdle()
        for (row in listOf(LOCAL_ROW, PEER_ROW, DOWN_ROW)) {
            onNodeWithContentDescription(row).assertExists()
        }
        // The offline node's own account of why, which is the one fact the count cannot carry.
        assertTrue(
            onAllNodesWithText("no route to host", substring = true).fetchSemanticsNodes().isNotEmpty(),
            "an unreachable machine was listed without saying why",
        )
    }

    // `resync` is the wire's documented recovery from a herd delta this client never received,
    // and no client code could send one: the message was encodable and never constructed outside
    // a test. The machine list is where a missing machine is noticed, so it is where the way out
    // of that belongs — on every layout, because a control that exists on one of three is dead on
    // the other two.
    @Test
    fun everyHerdLayoutCanAskTheNodeForTheWholeHerdAgain() {
        val asked = mutableListOf<String>()
        val layouts = listOf<Pair<String, @Composable () -> Unit>>(
            "portrait" to {
                Box(Modifier.size(420.dp, 900.dp)) {
                    HerdPortrait(HERD, ConnectionStatus.Live("full"), 0.0, 12.0, emptyList(), {}, null, onResync = { asked += "portrait" })
                }
            },
            "landscape" to {
                Box(Modifier.size(900.dp, 420.dp)) {
                    HerdLandscape(HERD, ConnectionStatus.Live("full"), 0.0, 12.0, emptyList(), {}, null, onResync = { asked += "landscape" })
                }
            },
            "sidebar" to {
                Box(Modifier.size(300.dp, 900.dp)) {
                    HerdSidebar(
                        HERD, ConnectionStatus.Live("full"), 0.0, 12.0, emptyList(), null, "this device", "full access", {}, {},
                        onResync = { asked += "sidebar" },
                    )
                }
            },
        )
        for ((name, layout) in layouts) runComposeUiTest {
            setContent { Themed { layout() } }
            waitForIdle()
            onNodeWithContentDescription("nodes online", substring = true).performClick()
            waitForIdle()
            onNodeWithContentDescription("Ask this node for the whole herd again").performClick()
            waitForIdle()
            assertTrue(name in asked, "the $name machine list could not ask for the herd again")
        }
    }

    // A control laid out with no width hugs its own label, and that is where a corner radius stops
    // reading as a corner: "Refresh" came out 54 dp wide against a 44 dp touch target, so `md` ate
    // both ends and the word sat on the curve. Every other action in the client is handed a width
    // by its caller; this one was not, and the report came back as "way too rounded".
    @Test
    fun theRefreshControlIsHandedAWidthRatherThanHuggingItsLabel() = runComposeUiTest {
        setContent { Themed { Box(Modifier.size(420.dp, 900.dp)) { HerdPortrait(HERD, ConnectionStatus.Live("full"), 0.0, 12.0, emptyList(), {}, null) } } }
        waitForIdle()
        onNodeWithContentDescription("nodes online", substring = true).performClick()
        waitForIdle()
        val rows = onNodeWithContentDescription(DOWN_ROW).getUnclippedBoundsInRoot()
        val button = onNodeWithContentDescription("Ask this node for the whole herd again")
            .getUnclippedBoundsInRoot()
        val row = rows.right - rows.left
        val refresh = button.right - button.left
        assertTrue(
            refresh >= row,
            "the refresh control measured $refresh beside a $row machine row, so its corners are most of it",
        )
    }

    // Three layouts share the pill, and a sheet that only opens on one of them is a control that
    // is dead on the other two.
    @Test
    fun everyHerdLayoutOpensAndClosesTheMachineList() {
        val layouts = listOf<Pair<String, @Composable () -> Unit>>(
            "portrait" to { Box(Modifier.size(420.dp, 900.dp)) { HerdPortrait(HERD, ConnectionStatus.Live("full"), 0.0, 12.0, emptyList(), {}, null) } },
            "landscape" to { Box(Modifier.size(900.dp, 420.dp)) { HerdLandscape(HERD, ConnectionStatus.Live("full"), 0.0, 12.0, emptyList(), {}, null) } },
            "sidebar" to {
                Box(Modifier.size(300.dp, 900.dp)) {
                    HerdSidebar(HERD, ConnectionStatus.Live("full"), 0.0, 12.0, emptyList(), null, "this device", "full access", {}, {})
                }
            },
        )
        for ((name, layout) in layouts) runComposeUiTest {
            setContent { Themed { layout() } }
            waitForIdle()
            onNodeWithContentDescription("nodes online", substring = true).performClick()
            waitForIdle()
            onNodeWithContentDescription(DOWN_ROW).assertExists().assertIsDisplayed()
            onNodeWithContentDescription("Close Machines").performClick()
            waitForIdle()
            assertTrue(
                onAllNodesWithContentDescription(DOWN_ROW).fetchSemanticsNodes().isEmpty(),
                "the $name layout could open the machine list but not close it",
            )
        }
    }
}
