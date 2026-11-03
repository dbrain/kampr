package dev.kampr.mosaic

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.v2.runComposeUiTest
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.height
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.terminal.TerminalSurfaces
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

// The report, a phone screenshot of an empty mosaic: the bar under the title was a band of nothing
// half again as tall as the title row. It was the row of pane chips, drawn with no chips in it.
@OptIn(ExperimentalTestApi::class)
class EmptySwitcherTest {
    private fun ComposeUiTest.switcher(fixture: Fixture) {
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
                        surfaces = TerminalSurfaces(),
                        landscape = false,
                        onHerd = {},
                        onAdd = {},
                    )
                }
            }
        }
        waitForIdle()
    }

    private fun ComposeUiTest.barHeight() = onNodeWithTag(SWITCHER_BAR_TAG).getUnclippedBoundsInRoot().height

    @Test
    fun anEmptyMosaicHasNoRowOfChipsUnderItsTitle() = runComposeUiTest {
        switcher(Fixture())
        assertEquals(SWITCHER_PORTRAIT, barHeight())
    }

    @Test
    fun aMosaicWithPanesStillHasItsChips() = runComposeUiTest {
        switcher(Fixture().apply { fourPanes() })
        assertTrue(barHeight() > SWITCHER_PORTRAIT + 40.dp)
    }
}
