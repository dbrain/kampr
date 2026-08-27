package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.v2.runSkikoComposeUiTest
import androidx.compose.ui.unit.Density
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.Segmented
import kotlin.test.Test
import kotlin.test.assertEquals

private val OPTIONS = listOf("Terminal", "Conversation")

// `IntrinsicSize.Max`, and that is the whole of the harness. Handed a width the control simply
// takes it -- its segments are weighted, so they split whatever they are given evenly and the
// selection cannot show. The number that moves is the one it *asks* for, which is what a wrapping
// parent measures it by and what `FlowRow` breaks its lines on.
@OptIn(ExperimentalTestApi::class)
private fun widthWith(selected: Int): Float {
    var width = 0f
    runSkikoComposeUiTest(Size(900f, 300f), Density(1f)) {
        setContent {
            CompositionLocalProvider(LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone, Ground.Dark)) {
                Box(Modifier.fillMaxSize()) {
                    Segmented(OPTIONS, selected, {}, Modifier.width(IntrinsicSize.Max), what = "view")
                }
            }
        }
        waitForIdle()
        width = onNodeWithContentDescription("${OPTIONS[0]} view").fetchSemanticsNode().boundsInRoot.left
        width = onNodeWithContentDescription("${OPTIONS[1]} view").fetchSemanticsNode().boundsInRoot.right - width
    }
    return width
}

// The selected segment is W700 and the rest are W500, so a control that measures only what it
// paints asks for a different width depending on which label is the bold one -- and "Conversation"
// in W700 is a bigger number than "Terminal" in W700. Anything that lays it out against the width
// left over then rewraps on the selection alone, which is the pane header dropping its view switch
// to a second row and sliding the segment out from under the thumb that just tapped it.
//
// Asserted here against the font Kampr actually ships. `PaneHeaderStabilityTest` is the same defect
// seen from the other end, and it only bit where the metrics were wide enough to force the wrap --
// which is why it failed on a bare CI runner for five releases and never once on a developer's
// machine.
class SegmentedWidthTest {
    @Test
    fun aSegmentedControlAsksForTheSameWidthWhicheverSegmentIsChosen() {
        assertEquals(
            widthWith(0),
            widthWith(1),
            "the control resized when the selection moved, so anything measured beside it reflows",
        )
    }
}
