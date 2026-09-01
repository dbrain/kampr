package dev.kampr.mosaic

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performMouseInput
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.rightClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.LocalManage
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.ManageIo
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.terminal.TerminalSurfaces
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private val NODE = NodeInfo(id = "01JNODE", name = "comingclean", kind = "local", online = true)

private val CELL = PaneInfo(
    id = "01JNODE/w1:p1",
    nodeId = "01JNODE",
    workspace = "kampr",
    cwd = "/home/dbrain/dev/kampr",
    agentStatus = "idle",
    cols = 74,
    rows = 30,
)

private class ManageSpy : ManageIo {
    val opened = mutableListOf<String>()
    override val enabled = true
    override fun openNew(paneId: String?) = Unit
    override fun openActions(paneId: String) {
        opened += paneId
    }
}

private fun cellTokens(): KamprTokens {
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    return KamprTokens(SoftTheme, fonts, typography(fonts, SoftTheme.label, TypeScale.Desk))
}

@Composable
private fun Cell(manage: ManageIo) {
    CompositionLocalProvider(
        LocalTokens provides cellTokens(),
        LocalPaneIo provides ArtboardIo,
        LocalManage provides manage,
    ) {
        Box(Modifier.size(520.dp, 340.dp)) {
            MosaicCell(
                PaneState(CELL.id, StyleTable()), CELL, NODE, focused = true,
                surfaces = TerminalSurfaces(), onFocus = {}, onRemove = {},
                modifier = Modifier.size(520.dp, 340.dp),
            )
        }
    }
}

// A cell is the one surface that must not take the long press: the grid under it selects text,
// which is why it opts out. The right-click half belongs to every surface, and losing one while
// fixing the other is the whole risk of moving the touch half into `Modifier.action`.
@OptIn(ExperimentalTestApi::class)
class MosaicCellActionsTest {
    @Test
    fun aRightClickOnACellStillOpensThePanesActions() = runComposeUiTest {
        val manage = ManageSpy()
        setContent { Cell(manage) }
        waitForIdle()
        onRoot().performMouseInput { rightClick(percentOffset(0.5f, 0.8f)) }
        waitForIdle()
        assertEquals(listOf(CELL.id), manage.opened, "a right-click in a cell opened ${manage.opened}")
    }

    @Test
    fun aLongPressInACellIsStillTheGridsAndOpensNothing() = runComposeUiTest {
        val manage = ManageSpy()
        setContent { Cell(manage) }
        waitForIdle()
        onRoot().performTouchInput { longClick(percentOffset(0.5f, 0.8f)) }
        waitForIdle()
        assertTrue(
            manage.opened.isEmpty(),
            "a long press took the gesture the grid selects with: opened ${manage.opened}",
        )
    }

    @Test
    fun theHeaderStillCarriesTheEllipsisThatTheCellRefusesToBe() = runComposeUiTest {
        val manage = ManageSpy()
        setContent { Cell(manage) }
        waitForIdle()
        onNodeWithContentDescription("Pane actions").performClick()
        waitForIdle()
        assertEquals(listOf(CELL.id), manage.opened, "the cell header's ellipsis opened ${manage.opened}")
    }
}
