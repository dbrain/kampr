package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertHeightIsAtLeast
import androidx.compose.ui.test.assertWidthIsAtLeast
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.Herd
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.HerdPortrait
import dev.kampr.shared.ui.HerdSidebar
import dev.kampr.shared.ui.LANDSCAPE_TOUCH
import dev.kampr.shared.ui.LocalManage
import dev.kampr.shared.ui.ManageIo
import dev.kampr.shared.wire.NodeInfo
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

// The operator, on 0.1.74: *"can we add a plus button next to each host so i can add a workspace to
// that host quickly without multiple steps — maybe it always creates a workspace as a 'quick open
// terminal' while the top level is the 'fancy new'"*.
//
// Two machines, because the whole point is that the + names one of them: the bar's + aims the sheet
// at whichever node this device is connected to, and picking a different one is a step of its own.
private val LOCAL = NodeInfo(id = "01JLOCAL", name = "comingclean", kind = "local", online = true)
private val PEER = NodeInfo(id = "01JPEER", name = "kobo", kind = "peer", online = true)

// A machine whose kampr answers and whose herdr does not. A manage op starts herdr (#324, #325), so
// this is the host the + is *most* use on — and the one an `online` test would have hidden it from.
private val COLD = NodeInfo(id = "01JCOLD", name = "coldbox", kind = "peer", online = false, reachable = true)

// Not paired, not reachable: nothing to create on, so nothing to press.
private val GONE = NodeInfo(id = "01JGONE", name = "gone", kind = "peer", online = false, reachable = false)

private class Spy(override val enabled: Boolean = true) : ManageIo {
    var quick: String? = null
    var sheet: Boolean = false

    override fun openNew(paneId: String?) {
        sheet = true
    }

    override fun openActions(paneId: String) = Unit
    override fun quickWorkspace(nodeId: String) {
        quick = nodeId
    }
}

@OptIn(ExperimentalTestApi::class)
class HerdQuickWorkspaceTest {
    @Test
    fun eachHostHasItsOwnPlusAndItCreatesOnThatHostWithNothingToFillIn() = runComposeUiTest {
        val spy = Spy()
        portrait(spy, LOCAL, PEER)
        onNodeWithContentDescription("New workspace on kobo").performClick()
        assertEquals(PEER.id, spy.quick, "the + beside a host did not create on that host")
        assertEquals(false, spy.sheet, "the host's + opened the sheet instead of creating")
    }

    // The bar's + is the fancy one and stays exactly as it was: the machine picker, the label, the
    // directory, the variables, the worktrees and the named sessions all live behind it.
    @Test
    fun theBarsPlusStillOpensTheSheet() = runComposeUiTest {
        val spy = Spy()
        portrait(spy, LOCAL, PEER)
        onNodeWithContentDescription("New workspace or session").performClick()
        assertEquals(true, spy.sheet)
        assertNull(spy.quick)
    }

    @Test
    fun aHostWhoseHerdrIsStoppedStillOffersItBecauseCreatingIsWhatStartsHerdr() = runComposeUiTest {
        portrait(Spy(), COLD)
        onNodeWithContentDescription("New workspace on coldbox").assertExists()
    }

    @Test
    fun aMachineNothingCanBeCreatedOnDoesNotOfferIt() = runComposeUiTest {
        portrait(Spy(), GONE)
        onNodeWithContentDescription("New workspace on gone").assertDoesNotExist()
    }

    // A read-only device, or a node with no manage ops: absent rather than present-and-failing.
    @Test
    fun aDeviceThatCannotManageIsNotOfferedIt() = runComposeUiTest {
        portrait(Spy(enabled = false), LOCAL)
        onNodeWithContentDescription("New workspace on comingclean").assertDoesNotExist()
    }

    // The sidebar is 296 dp with two long machine names in it, and `GlyphAction`'s `Modifier.size`
    // coerces to the constraint it is handed — so a header that overflows does not clip visibly, it
    // hands the whole deficit to the last child and the + measures 0 x 0. That has happened once.
    @Test
    fun theSidebarsHostPlusIsStillAPressableSizeBesideALongMachineName() = runComposeUiTest {
        val spy = Spy()
        setContent {
            CompositionLocalProvider(LocalTokens provides phoneTokens(), LocalManage provides spy) {
                Box(Modifier.size(1280.dp, 800.dp)) {
                    HerdSidebar(
                        herd = Herd(nodes = listOf(LOCAL, PEER), known = true),
                        connection = ConnectionStatus.Live("full"),
                        now = 0.0,
                        localRtt = 12.0,
                        triage = emptyList(),
                        activePaneId = null,
                        deviceName = "this desk",
                        deviceDetail = "paired",
                        onOpenPane = {},
                        onSettings = {},
                    )
                }
            }
        }
        onNodeWithContentDescription("New workspace on comingclean")
            .assertWidthIsAtLeast(LANDSCAPE_TOUCH)
            .assertHeightIsAtLeast(LANDSCAPE_TOUCH)
    }

    private fun androidx.compose.ui.test.ComposeUiTest.portrait(manage: ManageIo, vararg nodes: NodeInfo) {
        setContent {
            Themed(manage) {
                Box(Modifier.size(420.dp, 900.dp)) {
                    HerdPortrait(
                        Herd(nodes = nodes.toList(), known = true),
                        ConnectionStatus.Live("full"),
                        0.0,
                        12.0,
                        emptyList(),
                        {},
                        null,
                    )
                }
            }
        }
    }
}

@Composable
private fun Themed(manage: ManageIo, content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalTokens provides phoneTokens(), LocalManage provides manage, content = content)
}
