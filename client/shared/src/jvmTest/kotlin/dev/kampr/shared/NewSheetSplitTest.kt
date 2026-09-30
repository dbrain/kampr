package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.AppState
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.LocalMosaic
import dev.kampr.shared.ui.ManageLayer
import dev.kampr.shared.ui.MosaicSeed
import dev.kampr.shared.ui.Screen
import dev.kampr.shared.ui.Sheet
import dev.kampr.shared.wire.Wire
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

private const val SOURCE = "01JNODE/w1:p1"
private const val MADE = "01JNODE/w1:p2"

private const val MANAGER_HELLO =
    """{"t":"hello","protocol":1,"node_id":"01JNODE","node_name":"comingclean","build":"test",""" +
        """"role":"full","caps":{"manage":true},"security":{"tier":0,"passkeys":false}}"""

private fun herdOf(vararg ids: String) =
    """{"t":"herd","nodes":[{"id":"01JNODE","name":"comingclean","kind":"local"}],"panes":[""" +
        ids.joinToString(",") { """{"id":"$it","node_id":"01JNODE","workspace_id":"01JNODE/w1","workspace":"kampr"}""" } +
        "]}"

private fun KamprStore.take(frame: String) = accept(Wire.decode(frame) ?: error("undecodable: $frame"))

// "From a claude session pressed '+', chose 'right', chose '1/2' and it opened a new terminal
// completely separate from the current pane." Driven from the sheet's own buttons through
// `ManageLayer`, because the sheet is where the direction and the source pane are known and the
// layer is where the mosaic's availability is.
@OptIn(ExperimentalTestApi::class)
class NewSheetSplitTest {
    private fun split(chip: String, button: String, mosaic: Boolean, then: (AppState) -> Unit) = runComposeUiTest {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        val store = KamprStore()
        store.take(MANAGER_HELLO)
        store.take(herdOf(SOURCE))
        val state = AppState(scope, store, MemoryPrefs(), null)
        try {
            state.openPane(SOURCE)
            state.openSheet(Sheet.New("01JNODE", SOURCE))
            setContent {
                CompositionLocalProvider(
                    LocalTokens provides phoneTokens(),
                    LocalMosaic provides if (mosaic) ({ state.go(Screen.Mosaic) }) else null,
                ) {
                    Box(Modifier.size(420.dp, 900.dp)) {
                        ManageLayer(state, store.herd.value, Breakpoint.Portrait)
                    }
                }
            }
            press(chip)
            press(button)
            store.take("""{"t":"managed","op":"pane.split","ok":true,"id":"$MADE"}""")
            waitForIdle()
            store.take(herdOf(SOURCE, MADE))
            waitForIdle()
            then(state)
        } finally {
            scope.cancel()
        }
    }

    private fun ComposeUiTest.press(description: String) {
        onNodeWithContentDescription(description).performClick()
        waitForIdle()
    }

    @Test
    fun aSplitToTheRightOpensBothPanesSideBySide() = split("Split to the right", "Split right", mosaic = true) {
        assertEquals(Screen.Mosaic, it.screen)
        assertEquals(MosaicSeed(listOf(SOURCE, MADE), stacked = false), it.mosaicSeed)
    }

    @Test
    fun aSplitDownwardsOpensBothPanesStacked() = split("Split downwards", "Split down", mosaic = true) {
        assertEquals(Screen.Mosaic, it.screen)
        assertEquals(MosaicSeed(listOf(SOURCE, MADE), stacked = true), it.mosaicSeed)
    }

    @Test
    fun withNoMosaicASplitStillOpensThePaneItMade() = split("Split to the right", "Split right", mosaic = false) {
        assertEquals(MADE, (it.screen as Screen.Pane).paneId)
        assertNull(it.mosaicSeed)
    }
}
