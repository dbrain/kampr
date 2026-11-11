package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SkikoComposeUiTest
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.v2.runSkikoComposeUiTest
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.ui.LocalManage
import dev.kampr.shared.ui.PaneScreenDesktop
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.named
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ServerMsg
import kotlin.math.abs
import kotlin.test.Test
import kotlin.test.assertEquals

private const val PANE_ID = "01JNODE/w1:p1"

private object Blank : PaneSurfaces {
    @Composable override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = Box(modifier)

    @Composable override fun Zoom(pane: PaneState, modifier: Modifier) = Box(modifier.size(71.dp, 44.dp).named("Zoom"))
}

private class Look(
    val name: String,
    val agent: String? = null,
    val quiet: Boolean = false,
    val stale: Boolean = false,
    val unsent: Int = 0,
    val readOnly: Boolean = false,
)

private val LOOKS = listOf(
    Look("nothing"),
    Look("working", agent = "working"),
    Look("done", agent = "done"),
    Look("blocked", agent = "blocked"),
    Look("quiet", quiet = true),
    Look("quiet and working", agent = "working", quiet = true),
    Look("stale, unsent, read-only and blocked", agent = "blocked", stale = true, unsent = 3, readOnly = true),
)

private class Layout(val name: String, val width: Dp, val height: Dp, val desktop: Boolean = false, val landscape: Boolean = false)

private val LAYOUTS = listOf(
    Layout("portrait", 411.dp, 914.dp),
    Layout("narrow portrait", 360.dp, 800.dp),
    Layout("landscape", 914.dp, 411.dp, landscape = true),
    Layout("desktop", 1280.dp, 860.dp, desktop = true),
)

private val CONTROLS = listOf("Terminal view", "Conversation view")

private fun info(agent: String?) = PaneInfo(
    id = PANE_ID,
    nodeId = "01JNODE",
    workspace = "kampr",
    cwd = "/home/dbrain/dev/kampr",
    agent = "claude",
    agentStatus = agent ?: "idle",
    cols = 94,
    rows = 40,
    hasConversation = true,
)

private fun pane(look: Look) = PaneState(PANE_ID, StyleTable()).also { pane ->
    if (look.quiet) {
        pane.applyReset(ServerMsg.GridReset(PANE_ID, 4, 1, emptyList(), Cursor(0, 0, true), links = emptyList()))
        pane.noteStreamStopped()
    }
    pane.stale = look.stale
    repeat(look.unsent) { pane.noteUndelivered() }
}

@OptIn(ExperimentalTestApi::class)
private fun SkikoComposeUiTest.show(layout: Layout, look: Look) {
    setContent {
        CompositionLocalProvider(
            LocalTokens provides tokensFor(themeOf("soft"), TypeScale.Phone, Ground.Dark),
            LocalManage provides AllowManage,
        ) {
            Box(Modifier.fillMaxSize()) {
                if (layout.desktop) {
                    PaneScreenDesktop(pane(look), info(look.agent), PaneView.Terminal, Blank, look.readOnly, onView = {})
                } else {
                    PaneScreenMobile(pane(look), info(look.agent), PaneView.Terminal, Blank, layout.landscape, look.readOnly, onBack = {}, onView = {})
                }
            }
        }
    }
    waitForIdle()
}

private fun SemanticsNode.collect(into: MutableMap<String, Rect>) {
    if (layoutInfo.isPlaced) config.getOrNull(SemanticsProperties.ContentDescription)?.firstOrNull()?.let { into[it.substringBefore(" — ")] = boundsInRoot }
    children.forEach { it.collect(into) }
}

@OptIn(ExperimentalTestApi::class)
private fun SkikoComposeUiTest.described(): Map<String, Rect> = mutableMapOf<String, Rect>().also { onRoot(useUnmergedTree = true).fetchSemanticsNode().collect(it) }

private fun Map<String, Rect>.statuses(): List<Pair<String, Rect>> = filterKeys { key -> STATUS.any { key.startsWith(it) } || "did not reach" in key }.toList()

private val STATUS = listOf("This agent is", "Quiet", "Stale", "This device is read-only")

// The report: "working" showed at the very top of a phone while "quiet" pushed the
// Terminal/Conversation switch across. A status is something the eye goes looking for, so it has
// one home, and arriving or leaving never moves a control the thumb is about to press.
@OptIn(ExperimentalTestApi::class)
class PaneStatusPlacementTest {
    @Test
    fun noStatusMovesTheViewSwitch() {
        val moved = LAYOUTS.flatMap { layout ->
            val seen = LOOKS.map { look ->
                var bounds: Map<String, Rect> = emptyMap()
                runSkikoComposeUiTest(Size(layout.width.value, layout.height.value), Density(1f)) {
                    show(layout, look)
                    bounds = described()
                }
                look to bounds
            }
            val (_, first) = seen.first()
            seen.flatMap { (look, bounds) ->
                CONTROLS.mapNotNull { control ->
                    val was = first[control]
                    val now = bounds[control]
                    if (was == null || now == null || was != now) "${layout.name}, ${look.name}: $control was $was, now $now" else null
                }
            }
        }
        assertEquals(emptyList(), moved)
    }

    @Test
    fun theAgentsOwnStatusIsNeverTheOneCountedAway() {
        val hidden = LAYOUTS.flatMap { layout ->
            LOOKS.filter { it.agent != null }.mapNotNull { look ->
                var shown: List<Pair<String, Rect>> = emptyList()
                runSkikoComposeUiTest(Size(layout.width.value, layout.height.value), Density(1f)) {
                    show(layout, look)
                    shown = described().statuses()
                }
                "${layout.name}, ${look.name}".takeIf { shown.none { it.first.startsWith("This agent is") } }
            }
        }
        assertEquals(emptyList(), hidden)
    }

    @Test
    fun everyStatusSitsInTheSameRow() {
        val strays = LAYOUTS.flatMap { layout ->
            LOOKS.flatMap { look ->
                var shown: List<Pair<String, Rect>> = emptyList()
                runSkikoComposeUiTest(Size(layout.width.value, layout.height.value), Density(1f)) {
                    show(layout, look)
                    shown = described().statuses()
                }
                val row = shown.firstOrNull()?.second?.center?.y ?: return@flatMap emptyList()
                shown.filter { abs(it.second.center.y - row) > 2f }.map { "${layout.name}, ${look.name}: ${it.first} at ${it.second}" }
            }
        }
        assertEquals(emptyList(), strays)
    }
}
