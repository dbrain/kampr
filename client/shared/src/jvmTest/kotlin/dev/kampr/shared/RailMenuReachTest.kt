package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.height
import androidx.compose.ui.unit.width
import dev.kampr.shared.model.Herd
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.HerdRail
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LANDSCAPE_TOUCH
import dev.kampr.shared.ui.LocalManage
import dev.kampr.shared.ui.ManageIo
import dev.kampr.shared.ui.MenuAnchor
import dev.kampr.shared.ui.PaneRow
import dev.kampr.shared.ui.RAIL_WIDTH
import dev.kampr.shared.ui.SIDEBAR_WIDTH
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import kotlin.test.Test
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

private val NODE = NodeInfo(id = "01JNODE", name = "comingclean", kind = "local", online = true)

private val RAIL_PANE = PaneInfo(
    id = "01JNODE/w1:p1",
    nodeId = "01JNODE",
    workspaceId = "01JNODE/w1",
    workspace = "kampr",
    tab = "1",
    cwd = "/home/dbrain/dev/kampr",
    agent = "claude",
    agentStatus = "working",
    cols = 94,
    rows = 40,
)

private val HERD = Herd(nodes = listOf(NODE), panes = listOf(RAIL_PANE), known = true)

private object CanManage : ManageIo {
    override val enabled = true
    override fun openNew(paneId: String?) = Unit
    override fun openActions(paneId: String) = Unit
    override fun openMenu(paneId: String, at: MenuAnchor?) = Unit
}

@androidx.compose.runtime.Composable
private fun Rail() {
    CompositionLocalProvider(LocalTokens provides phoneTokens(), LocalManage provides CanManage) {
        Box(Modifier.size(RAIL_WIDTH, 700.dp)) {
            HerdRail(HERD, now = 0.0, activePaneId = null, deviceName = "this phone", deviceDetail = "paired", {}, {}, {})
        }
    }
}

// The sigil measured on its own terms. Inside the rail the text node is stretched to the tile it
// is centred in, so its bounds there say nothing about how much width the two characters need.
@androidx.compose.runtime.Composable
private fun Sigil() {
    CompositionLocalProvider(LocalTokens provides phoneTokens()) {
        Box(Modifier.width(IntrinsicSize.Min)) {
            KText("K1", Kampr.tokens.type.pill, Kampr.tokens.color.text)
        }
    }
}

@androidx.compose.runtime.Composable
private fun Row() {
    CompositionLocalProvider(LocalTokens provides phoneTokens(), LocalManage provides CanManage) {
        Box(Modifier.size(SIDEBAR_WIDTH, 200.dp)) {
            PaneRow(RAIL_PANE, now = 0.0, active = false) {}
        }
    }
}

// **Why the collapsed rail carries no "…", written down where the next person will look for it.**
//
// `PaneActionsGesture`'s rule is that the right-click is a shortcut and never the only way in,
// because a right-click is silent to a screen reader — "behind the '…' glyph on every surface that
// carries this, and behind the semantic long press where there is no room for a glyph". The rail
// is the surface that clause was written for, and this is the measurement rather than the taste:
//
// Measured, on the composed rail: the rail is 56 dp wide and a tile is inset 5 dp each side, so a
// tile is **46 x 44 dp**. Inside it, stacked and centred, are the two-character sigil that is the
// *only* thing naming the pane — **16 dp of ink** — and the 7 dp status mark. `PaneMenuAction` is a
// `GlyphAction` whose smallest honest target is `LANDSCAPE_TOUCH`, 36 dp: the two-thumb floor the
// whole client is built to, around a 28 dp chip. That leaves **10 dp** for a name that needs 16, so
// the glyph does not go beside the sigil. Laid *over* the tile instead it covers the sigil at
// exactly the moment the pointer is on it, and takes the middle of the tile's own press with it —
// a click on a hovered tile would open the menu rather than the pane, which is the one thing a rail
// exists to do. Widening the rail to fit both is the sidebar coming back, which is what the rail
// was made to give away.
//
// So the rail keeps three ways in and no glyph: the right-click herdr itself uses, the long press
// (`Modifier.action`'s `onLongClick`, which a held mouse button fires as well as a finger), and —
// the visible one — the labelled "Expand the sidebar" control at the top of the rail, one press
// away from the `PaneRow` that does carry the glyph. A collapsed view of a surface is allowed to
// reach the expanded one for an affordance it has no room for; what it may not do is have no
// visible route at all, and it does not.
@OptIn(ExperimentalTestApi::class)
private fun sigilInk(): Dp {
    var ink = 0.dp
    runComposeUiTest {
        setContent { Sigil() }
        ink = onNodeWithText("K1").getUnclippedBoundsInRoot().width
    }
    return ink
}

@OptIn(ExperimentalTestApi::class)
class RailMenuReachTest {
    @Test
    fun aRailTileHasNoRoomForTheMenuGlyphBesideTheOnlyThingThatNamesThePane() = runComposeUiTest {
        setContent { Rail() }
        val tile = onNodeWithContentDescription("Open ", substring = true).getUnclippedBoundsInRoot()
        assertTrue(
            tile.width - LANDSCAPE_TOUCH < sigilInk(),
            "a ${tile.width} tile now leaves ${tile.width - LANDSCAPE_TOUCH} beside a " +
                "$LANDSCAPE_TOUCH glyph, which is more than the ${sigilInk()} the sigil needs — the " +
                "rail has room for the menu glyph now, so give it one",
        )
        assertTrue(tile.height <= 44.dp, "the tile grew to ${tile.height}")
    }

    @Test
    fun aRailTileReachesItsMenuByLongPressAndTheRailReachesTheGlyphByExpanding() = runComposeUiTest {
        setContent { Rail() }
        val tile = onNodeWithContentDescription("Open ", substring = true).fetchSemanticsNode()
        assertNotNull(
            tile.config.getOrNull(SemanticsActions.OnLongClick),
            "the rail tile's only reader-reachable way into the pane menu is gone",
        )
        onNodeWithContentDescription("Expand the sidebar").assertExists()
    }

    // The other end of that route, and the reason the rail is allowed to send an operator down it.
    @Test
    fun theExpandedSidebarRowCarriesTheGlyphTheRailHasNoRoomFor() = runComposeUiTest {
        setContent { Row() }
        onNodeWithContentDescription("Pane menu").assertExists()
    }
}
