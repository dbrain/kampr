package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsNodeInteractionsProvider
import androidx.compose.ui.test.SkikoComposeUiTest
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.v2.runSkikoComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.DpRect
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.Herd
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.ThemeMode
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.on
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.AppearanceScreen
import dev.kampr.shared.ui.COLUMN_MAX
import dev.kampr.shared.ui.HerdLandscape
import dev.kampr.shared.ui.SetupScreen
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.Security
import kotlin.math.abs
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private fun tokens() = themeOf("soft").on(Ground.Dark).let { spec ->
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    KamprTokens(spec, fonts, typography(fonts, spec.label, TypeScale.Desk))
}

@Composable
private fun Themed(content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalTokens provides tokens(), content = content)
}

private val NODES = listOf(
    NodeInfo(id = "01JA", name = "comingclean", kind = "local", build = "0.1.13"),
    NodeInfo(id = "01JB", name = "haymaker", kind = "peer", herdrVersion = "0.9.1"),
    NodeInfo(id = "01JC", name = "sungrow-pi", kind = "peer", herdrVersion = "0.9.1"),
)

private fun pane(node: String, workspace: String, tab: String, agent: String?, status: String, cwd: String) = PaneInfo(
    id = "$node/$workspace:$tab",
    nodeId = node,
    workspaceId = "$node/$workspace",
    tabId = "$node/$workspace:t$tab",
    workspace = workspace,
    tab = tab,
    cwd = cwd,
    agent = agent,
    agentStatus = status,
    updatedAt = "2026-08-24T10:00:00Z",
)

private val PANES = listOf(
    pane("01JA", "kampr", "1", "claude", "blocked", "/home/dbrain/dev/kampr"),
    pane("01JA", "kampr", "2", "claude", "working", "/home/dbrain/dev/kampr/client/shared"),
    pane("01JA", "herdr", "1", null, "unknown", "/home/dbrain/dev/herdr"),
    pane("01JB", "infra", "1", "codex", "done", "/srv/infra"),
    pane("01JB", "infra", "2", null, "unknown", "/srv"),
    pane("01JC", "solar", "1", "claude", "working", "/opt/sungrow"),
)

private val HERD = Herd(nodes = NODES, panes = PANES, known = true)
private val ONE_MACHINE = Herd(nodes = NODES.take(1), panes = PANES.take(3), known = true)

private const val NODE_ROW = "comingclean, this machine, online, kampr 0.1.13"
private const val LADDER_LABEL = "Optional, in any order"
private const val GREETING = "You're already in."

// `runComposeUiTest` clamps its content to a 1024 dp window, which is narrower than every window
// this file is about, so the sizes have to be the scene's rather than a modifier's.
@OptIn(ExperimentalTestApi::class)
private fun window(width: Dp, height: Dp, block: SkikoComposeUiTest.() -> Unit) =
    runSkikoComposeUiTest(Size(width.value, height.value), Density(1f)) { block() }

@OptIn(ExperimentalTestApi::class)
private fun SemanticsNodeInteractionsProvider.cards(): List<DpRect> {
    val found = onAllNodesWithContentDescription("Open ", substring = true)
    return found.fetchSemanticsNodes().indices.map { found[it].getUnclippedBoundsInRoot() }
}

private fun List<DpRect>.lefts(): List<Dp> = map { it.left }.distinct().sorted()

@Composable
private fun Herd(herd: Herd) {
    Box(Modifier.fillMaxSize()) {
        HerdLandscape(herd, ConnectionStatus.Live("full"), 0.0, 4.0, emptyList(), {}, null)
    }
}

@Composable
private fun Setup() {
    Box(Modifier.fillMaxSize()) {
        SetupScreen(
            status = null,
            security = Security(),
            running = true,
            endpoint = null,
            nodes = NODES,
            pairingCode = null,
            pairingError = null,
            onConnect = {},
            onPairingCode = {},
            onDevices = {},
            onAppearance = {},
            onNotifications = {},
            wide = true,
        )
    }
}

// A window three times as wide as the one a layout was drawn for has nowhere to put the extra width
// but into the columns themselves. The herd arrived as two 1700 dp cards with the machine name
// against one edge of the screen and the clock against the other, and settings as two 520 dp
// measures a thousand dp apart.
@OptIn(ExperimentalTestApi::class)
class WideWindowTest {
    @Test
    fun aPaneCardStopsGrowingAtTheWidthItsOwnContentNeeds() {
        for (width in listOf(1280.dp, 1920.dp, 3440.dp)) window(width, 560.dp) {
            setContent { Themed { Herd(HERD) } }
            waitForIdle()
            val card = cards().first()
            val measured = card.right - card.left
            assertTrue(
                measured <= COLUMN_MAX,
                "a $width window stretched a pane card to $measured, past the $COLUMN_MAX its content ends at",
            )
        }
    }

    @Test
    fun theColumnsSitTogetherInTheMiddleRatherThanAtOppositeEdges() = window(3440.dp, 560.dp) {
        setContent { Themed { Herd(HERD) } }
        waitForIdle()
        val lefts = cards().lefts()
        assertTrue(lefts.size >= 2, "3440 dp produced one column of cards, so there is nothing to sit together")
        val band = lefts.last() + COLUMN_MAX - lefts.first()
        assertTrue(
            band <= COLUMN_MAX * lefts.size + 40.dp,
            "the columns spanned $band across ${lefts.size} columns, which is a canyon rather than a band",
        )
        val leading = lefts.first()
        val trailing = 3440.dp - (lefts.last() + COLUMN_MAX)
        assertTrue(
            abs((leading - trailing).value) <= 24f,
            "the band sits $leading from the left and $trailing from the right, so it is not centred",
        )
    }

    // Two columns at 1000 dp and two at 3400 was the defect: the count has to answer the window.
    @Test
    fun theNumberOfColumnsAnswersTheWindow() {
        val counted = mutableMapOf<Dp, Int>()
        for (width in listOf(640.dp, 1280.dp, 3440.dp)) window(width, 560.dp) {
            setContent { Themed { Herd(HERD) } }
            waitForIdle()
            counted[width] = cards().lefts().size
        }
        assertEquals(1, counted[640.dp], "a window with room for one measure was still split in two")
        assertEquals(2, counted[1280.dp], "1280 dp did not fit the two columns it has room for")
        assertTrue(
            counted[3440.dp]!! > counted[1280.dp]!!,
            "3440 dp laid out ${counted[3440.dp]} columns, the same as 1280 dp, so the count ignores the window",
        )
    }

    // More columns than there are machines is an empty column, and an empty column is the canyon
    // again — invisible in the cards, and obvious in where the one real column ends up sitting.
    @Test
    fun anUltrawideNeverOpensAColumnWithNothingInIt() = window(3440.dp, 560.dp) {
        setContent { Themed { Herd(ONE_MACHINE) } }
        waitForIdle()
        val card = cards().first()
        val middle = (card.left + card.right) / 2
        assertTrue(
            abs(middle.value - 1720f) <= 12f,
            "the only machine sits centred on $middle, so the layout kept columns open for machines it does not have",
        )
    }

    // Settings was already two measures of 520 dp, but they were pinned to the left edge of each
    // half of the window, which on an ultrawide is a thousand dp of nothing between them.
    @Test
    fun aWideDesktopPutsTheTwoSettingsMeasuresBesideEachOtherNotAtOppositeEdges() = window(3440.dp, 1440.dp) {
        setContent { Themed { Setup() } }
        waitForIdle()
        val machines = onNodeWithContentDescription(NODE_ROW).getUnclippedBoundsInRoot()
        val ladder = onNodeWithText(LADDER_LABEL).getUnclippedBoundsInRoot()
        assertTrue(machines.left > ladder.right, "the two settings columns are not side by side")
        val band = machines.right - ladder.left
        assertTrue(
            band <= COLUMN_MAX * 2 + 60.dp,
            "the two measures span $band between them, which is a canyon rather than a pair of columns",
        )
        val middle = (ladder.left + machines.right) / 2
        assertTrue(
            abs(middle.value - 1720f) <= 40f,
            "the pair is centred on $middle in a 3440 dp window, so it is pinned to an edge rather than the middle",
        )
        val intro = onNodeWithText(GREETING).getUnclippedBoundsInRoot()
        assertTrue(
            abs((intro.left - ladder.left).value) <= 20f,
            "the greeting starts at ${intro.left} and the column it belongs to at ${ladder.left}, so it was left behind",
        )
    }

    @Test
    fun aThemeCardStopsGrowingAtTheWidthItsOwnContentNeeds() = window(3440.dp, 1440.dp) {
        setContent {
            Themed {
                Box(Modifier.fillMaxSize()) {
                    AppearanceScreen(themeOf("soft").id, ThemeMode.Dark, {}, {}, {})
                }
            }
        }
        waitForIdle()
        val card = onNodeWithContentDescription("Soft native theme", substring = true).getUnclippedBoundsInRoot()
        val measured = card.right - card.left
        assertTrue(
            measured <= COLUMN_MAX,
            "a 3440 dp window stretched a theme card to $measured, past the $COLUMN_MAX it needs",
        )
    }
}
