package dev.kampr.mosaic

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.click
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.wire.PaneInfo
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private class Composed : PaneSurfaces {
    val terminals = mutableStateListOf<String>()
    val keyRows = mutableStateListOf<String>()

    @Composable
    override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) {
        terminals += pane.id
        Box(modifier)
    }

    @Composable
    override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable
    override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) {
        keyRows += pane.id
        Box(modifier)
    }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.switcher(fixture: Fixture, surfaces: PaneSurfaces, landscape: Boolean = false) {
    setContent {
        CompositionLocalProvider(
            LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone, Ground.Dark),
            LocalPaneIo provides ArtboardIo,
        ) {
            Box(Modifier.fillMaxSize()) {
                MosaicSwitcher(
                    store = fixture.store,
                    mosaic = fixture.mosaic,
                    herd = fixture.store.herd.value,
                    surfaces = surfaces,
                    landscape = landscape,
                    onHerd = {},
                    onAdd = {},
                )
            }
        }
    }
    waitForIdle()
}

// Two panes on one phone screen: the chips choose which two, the focus chooses which one types.
@OptIn(ExperimentalTestApi::class)
class MosaicPairTest {
    private val fixture = Fixture().apply { fourPanes() }
    private val mosaic = fixture.mosaic

    @Test
    fun pairingShowsTheFocusedPaneAndItsNeighbour() {
        assertEquals(listOf(CLAUDE), mosaic.shown)
        mosaic.pair(true)
        assertEquals(listOf(CLAUDE, CODEX), mosaic.shown)
        mosaic.pair(false)
        assertEquals(listOf(CLAUDE), mosaic.shown)
    }

    // A chip already on screen only moves the focus; one that is not takes the half the focus is
    // not in, so what was being typed into stays where it was.
    @Test
    fun aChipReplacesTheHalfThatIsNotFocused() {
        mosaic.pair(true)
        for ((tap, shown) in listOf(
            SUNGROW to listOf(CLAUDE, SUNGROW),
            CLAUDE to listOf(CLAUDE, SUNGROW),
            DOGE to listOf(CLAUDE, DOGE),
            CODEX to listOf(CODEX, DOGE),
        )) {
            mosaic.focus(tap)
            assertEquals(shown, mosaic.shown, "after $tap")
            assertEquals(tap, mosaic.focused)
        }
    }

    @Test
    fun onePaneFallsBackToOne() {
        mosaic.pair(true)
        for (id in listOf(CODEX, SUNGROW, DOGE)) mosaic.remove(id)
        assertEquals(listOf(CLAUDE), mosaic.shown)
    }

    @Test
    fun removingTheFocusedHalfFocusesTheOtherAndRefills() {
        mosaic.pair(true)
        mosaic.focus(SUNGROW)
        mosaic.remove(SUNGROW)
        assertEquals(CLAUDE, mosaic.focused)
        assertEquals(2, mosaic.shown.size)
        assertTrue(CLAUDE in mosaic.shown)
    }

    @Test
    fun thePairingIsRemembered() {
        val prefs = MemoryPrefs()
        MosaicState(prefs, fixture.connection).apply {
            restore()
            pair(true)
        }
        assertTrue(MosaicState(prefs, fixture.connection).apply { restore() }.paired)
    }

    @Test
    fun theBarTogglesTwoCellsAndOnlyTheFocusedOneHasTheKeyRow() {
        for (landscape in listOf(false, true)) {
            runComposeUiTest {
                val fixture = Fixture().apply { fourPanes() }
                val surfaces = Composed()
                switcher(fixture, surfaces, landscape)
                onNodeWithContentDescription("Show two panes at once").performClick()
                waitForIdle()
                assertTrue(fixture.mosaic.paired, "landscape=$landscape")
                surfaces.terminals.clear()
                surfaces.keyRows.clear()
                fixture.mosaic.focus(CODEX)
                waitForIdle()
                assertEquals(setOf(CLAUDE, CODEX), surfaces.terminals.toSet(), "landscape=$landscape")
                assertEquals(setOf(CODEX), surfaces.keyRows.toSet(), "landscape=$landscape")

                // The first half is top in portrait and left in landscape; a touch there is a focus.
                surfaces.keyRows.clear()
                onRoot().performTouchInput { click(Offset(width * 0.25f, height * 0.4f)) }
                waitForIdle()
                assertEquals(CLAUDE, fixture.mosaic.focused, "landscape=$landscape")
                assertEquals(setOf(CLAUDE), surfaces.keyRows.toSet(), "landscape=$landscape")
            }
        }
    }

    @Test
    fun oneHalfCannotBeAskedForWithOnePane() = runComposeUiTest {
        for (id in listOf(CODEX, SUNGROW, DOGE)) mosaic.remove(id)
        switcher(fixture, Composed())
        assertEquals(
            0,
            onAllNodes(hasContentDescription("Show two panes at once")).fetchSemanticsNodes().size,
        )
    }
}
