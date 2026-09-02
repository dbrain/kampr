package dev.kampr.shared

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertHeightIsAtLeast
import androidx.compose.ui.test.assertWidthIsAtLeast
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.isFocused
import androidx.compose.ui.test.requestFocus
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.AgentStatus
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.TriageItem
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.on
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.BottomSheet
import dev.kampr.shared.ui.HerdPortrait
import dev.kampr.shared.ui.SheetHeader
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.FallbackSurfaces
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.TOUCH
import dev.kampr.shared.ui.statusShape
import dev.kampr.shared.ui.statusWord
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private val NODE = NodeInfo(id = "01JNODE", name = "comingclean", kind = "local", online = true)

private fun pane(id: String, status: String, workspace: String) = PaneInfo(
    id = "01JNODE/$id",
    nodeId = "01JNODE",
    workspace = workspace,
    cwd = "/home/dbrain/dev/kampr",
    agent = "claude",
    agentStatus = status,
    cols = 74,
    rows = 30,
)

private val HERD = Herd(
    nodes = listOf(NODE),
    panes = listOf(
        pane("w1:p1", "blocked", "kampr"),
        pane("w2:p1", "working", "herdr"),
        pane("w3:p1", "done", "notes"),
        pane("w4:p1", "idle", "scratch"),
    ),
    known = true,
)

// Semantics do not need a real typeface, and loading five families off disk to assert on a string
// is what makes a fast test slow.
private fun tokens() = themeOf("soft").on(Ground.Dark).let { spec ->
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    KamprTokens(spec, fonts, typography(fonts, spec.label, TypeScale.Phone))
}

@Composable
private fun Themed(content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalTokens provides tokens(), content = content)
}

@OptIn(ExperimentalTestApi::class)
class HerdAccessibilityTest {
    // Four statuses, four silhouettes. If two of these ever collapse onto the same shape the only
    // channel left is hue, which is the defect this replaced.
    @Test
    fun everyStatusHasAShapeOfItsOwn() {
        val shapes = AgentStatus.entries.filter { it != AgentStatus.Unknown }.map(::statusShape)
        assertEquals(shapes.size, shapes.toSet().size, "two statuses share a silhouette: $shapes")
        assertTrue(AgentStatus.entries.all { statusWord(it).isNotBlank() })
    }

    @Test
    fun theHerdListSaysWhatEachAgentIsDoing() = runComposeUiTest {
        setContent {
            Themed { HerdPortrait(HERD, ConnectionStatus.Live("full"), 0.0, 12.0, emptyList(), {}, null) }
        }
        for (word in listOf("Blocked", "Working", "Done", "Idle")) {
            assertTrue(
                onAllNodesWithContentDescription(word, substring = true).fetchSemanticsNodes().isNotEmpty(),
                "no pane in the herd announces itself as $word",
            )
        }
    }

    // A blocked agent turns up while the operator is somewhere else on the screen, which is the
    // whole case for a live region rather than a label.
    @Test
    fun aBlockedAgentInterrupts() = runComposeUiTest {
        val triage = listOf(TriageItem(HERD.panes.first(), "Approve edit to server.ts"))
        setContent {
            Themed { HerdPortrait(HERD, ConnectionStatus.Live("full"), 0.0, 12.0, triage, {}, {}) }
        }
        onNodeWithContentDescription("Needs you", substring = true)
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.LiveRegion, LiveRegionMode.Assertive))
    }

    // A sheet that can only be dismissed by finding its scrim or its cross is a sheet a keyboard
    // cannot leave.
    @Test
    fun escapeClosesASheet() = runComposeUiTest {
        var open = true
        setContent {
            Themed {
                if (open) {
                    BottomSheet(Breakpoint.Portrait, onDismiss = { open = false }) {
                        SheetHeader("Actions", null, null, { open = false })
                    }
                }
            }
        }
        waitForIdle()
        assertTrue(
            onAllNodes(isFocused()).fetchSemanticsNodes().isNotEmpty(),
            "opening a sheet left focus outside it, so the first Escape would go nowhere",
        )
        onNodeWithContentDescription("Close Actions").assertExists().requestFocus()
        waitForIdle()
        onNodeWithContentDescription("Close Actions").performKeyInput { pressKey(Key.Escape) }
        waitForIdle()
        assertTrue(!open, "Escape did not close the sheet")
    }

    @Test
    fun theBackChevronIsNamedForWhereItGoesAndBigEnoughToHit() = runComposeUiTest {
        setContent {
            Themed {
                PaneScreenMobile(
                    pane = PaneState("01JNODE/w1:p1", StyleTable()),
                    info = HERD.panes.first(),
                    view = PaneView.Terminal,
                    surfaces = FallbackSurfaces,
                    landscape = false,
                    readOnly = false,
                    onBack = {},
                    onView = {},
                )
            }
        }
        onNodeWithContentDescription("Back to the herd")
            .assertHeightIsAtLeast(TOUCH)
            .assertWidthIsAtLeast(TOUCH)
    }

}
