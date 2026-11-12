package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.click
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.v2.runSkikoComposeUiTest
import androidx.compose.ui.unit.Density
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.ui.LocalManage
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.TOUCH
import dev.kampr.shared.wire.PaneInfo
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val PANE = "01JNODE/w1:p1"

private object Empty : PaneSurfaces {
    @Composable override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = Box(modifier)
}

// The report: a hefty gap between the back chevron and the title. The title comes in to sit just
// past the chevron, and the strip the gap used to be still takes a thumb back.
@OptIn(ExperimentalTestApi::class)
class PaneBackGapTest {
    @Test
    fun theTitleSitsBesideTheChevronAndTheTargetStaysWhole() {
        var backs = 0
        runSkikoComposeUiTest(Size(411f, 914f), Density(1f)) {
            setContent {
                CompositionLocalProvider(
                    LocalTokens provides tokensFor(themeOf("soft"), TypeScale.Phone, Ground.Dark),
                    LocalManage provides AllowManage,
                ) {
                    Box(Modifier.fillMaxSize()) {
                        PaneScreenMobile(
                            PaneState(PANE, StyleTable()),
                            PaneInfo(id = PANE, nodeId = "01JNODE", workspace = "kampr", agent = "claude", agentStatus = "working", cols = 94, rows = 40, hasConversation = true),
                            PaneView.Terminal,
                            Empty,
                            landscape = false,
                            readOnly = false,
                            onBack = { backs++ },
                            onView = {},
                        )
                    }
                }
            }
            waitForIdle()
            val back = onNode(SemanticsMatcher.expectValue(SemanticsProperties.ContentDescription, listOf("Back to the herd")), useUnmergedTree = true)
                .fetchSemanticsNode().boundsInRoot
            val title = onNode(SemanticsMatcher.keyIsDefined(SemanticsProperties.Heading), useUnmergedTree = true)
                .fetchSemanticsNode().boundsInRoot
            assertTrue(back.width >= TOUCH.value && back.height >= TOUCH.value, "back target $back")
            assertTrue(title.left - back.left <= 56f, "title starts ${title.left - back.left} dp past the back target's edge")

            val y = back.center.y
            listOf(back.left + 4f, back.center.x + 10f, back.right - 2f).forEach { x ->
                onRoot().performTouchInput { click(Offset(x, y)) }
                waitForIdle()
            }
            assertEquals(3, backs)
        }
    }
}
