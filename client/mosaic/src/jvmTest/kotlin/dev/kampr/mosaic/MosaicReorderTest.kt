package dev.kampr.mosaic

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.test.swipe
import androidx.compose.ui.text.font.FontFamily
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.terminal.TerminalSurfaces
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private fun testTokens(): KamprTokens {
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    return KamprTokens(SoftTheme, fonts, typography(fonts, SoftTheme.label, TypeScale.Desk))
}

private fun SemanticsNodeInteraction.customAction(label: String): CustomAccessibilityAction {
    val actions = fetchSemanticsNode().config
        .getOrElseNullable(SemanticsActions.CustomActions) { null }
        .orEmpty()
    return actions.firstOrNull { it.label == label }
        ?: error("no custom action \"$label\" — offered ${actions.map { it.label }}")
}

@Composable
private fun Screen(fixture: Fixture) {
    CompositionLocalProvider(LocalTokens provides testTokens(), LocalPaneIo provides ArtboardIo) {
        Box(Modifier.fillMaxSize()) {
            MosaicScreen(
                store = fixture.store,
                mosaic = fixture.mosaic,
                herd = fixture.store.herd.value,
                connectionStatus = ConnectionStatus.Live("full"),
                build = "0.1.0",
                surfaces = TerminalSurfaces(),
                onHerd = {},
                onAdd = {},
            )
        }
    }
}

// The layout you got was the layout you kept. Dragging is the obvious way to change it and the one
// a screen reader cannot use, so both paths are here or neither is done.
@OptIn(ExperimentalTestApi::class)
class MosaicReorderTest {
    @Test
    fun aCellCanBeMovedWithoutADragAtAll() = runComposeUiTest {
        val fixture = Fixture().apply { fourPanes() }
        setContent { Screen(fixture) }

        val grip = onNodeWithContentDescription("Reorder kampr", substring = true)
        grip.customAction("Move this pane later").action()
        waitForIdle()
        assertEquals(listOf(CODEX, CLAUDE, SUNGROW, DOGE), fixture.mosaic.panes)

        onNodeWithContentDescription("Reorder kampr", substring = true)
            .customAction("Move this pane earlier").action()
        waitForIdle()
        assertEquals(listOf(CLAUDE, CODEX, SUNGROW, DOGE), fixture.mosaic.panes)
    }

    @Test
    fun draggingACellOntoAnotherSwapsThem() = runComposeUiTest {
        val fixture = Fixture().apply { fourPanes() }
        setContent { Screen(fixture) }

        val before = fixture.mosaic.panes
        onNodeWithContentDescription("Reorder kampr", substring = true).performTouchInput {
            swipe(center, center + androidx.compose.ui.geometry.Offset(600f, 0f), durationMillis = 300)
        }
        waitForIdle()
        assertTrue(fixture.mosaic.panes != before, "the drag did nothing: ${fixture.mosaic.panes}")
        assertEquals(before.toSet(), fixture.mosaic.panes.toSet(), "a drag must not add or drop a pane")
        assertEquals(CLAUDE, fixture.mosaic.panes[1], "the dragged cell took the place it was dropped on")
    }

    // The phone has no Save control at all, so a layout rearranged with a thumb could never be
    // kept. It gets one, and only when there is something to keep.
    @Test
    fun theSwitcherOffersMovesAndASaveOnlyWhenTheLayoutHasChanged() = runComposeUiTest {
        val fixture = Fixture().apply { fourPanes() }
        fixture.mosaic.save()
        setContent {
            CompositionLocalProvider(LocalTokens provides testTokens(), LocalPaneIo provides ArtboardIo) {
                Box(Modifier.fillMaxSize()) {
                    MosaicSwitcher(
                        store = fixture.store,
                        mosaic = fixture.mosaic,
                        herd = fixture.store.herd.value,
                        surfaces = TerminalSurfaces(),
                        landscape = false,
                        onHerd = {},
                        onAdd = {},
                    )
                }
            }
        }
        onAllNodesWithContentDescription("Save this layout on this device").assertCountEquals(0)

        onNodeWithContentDescription("Show kampr", substring = true)
            .customAction("Move this pane later").action()
        waitForIdle()
        assertEquals(CODEX, fixture.mosaic.panes.first())
        onNodeWithContentDescription("Save this layout on this device").assertExists()
    }
}
