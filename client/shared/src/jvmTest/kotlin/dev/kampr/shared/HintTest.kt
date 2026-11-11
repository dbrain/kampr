package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.click
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performMouseInput
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.v2.runComposeUiTest
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.ui.GlyphTarget
import dev.kampr.shared.ui.HINT_HOVER_MS
import dev.kampr.shared.ui.HINT_LINGER_MS
import dev.kampr.shared.ui.KamprIcons
import kotlin.test.Test
import kotlin.test.assertEquals

private const val LABEL = "Close pane"

@OptIn(ExperimentalTestApi::class)
class HintTest {
    private val tokens = tokensFor(themeOf("soft"), TypeScale.Phone, Ground.Dark)

    private fun ComposeUiTest.control(onClick: () -> Unit = {}, onLongClick: (() -> Unit)? = null) {
        setContent {
            CompositionLocalProvider(LocalTokens provides tokens) {
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    GlyphTarget(KamprIcons.cross, LABEL, tokens.color.dim, onClick, onLongClick = onLongClick)
                }
            }
        }
    }

    private fun ComposeUiTest.hintShown(): Boolean = onAllNodes(hasText(LABEL)).fetchSemanticsNodes().isNotEmpty()

    @Test
    fun aRestingMouseShowsTheLabelAfterTheDelayAndLeavingHidesIt() = runComposeUiTest {
        control()
        onNodeWithContentDescription(LABEL).performMouseInput { enter(center) }
        mainClock.advanceTimeBy(HINT_HOVER_MS / 2)
        assertEquals(false, hintShown(), "the hint waits before it shows")
        mainClock.advanceTimeBy(HINT_HOVER_MS)
        onNodeWithText(LABEL).assertExists()
        onNodeWithContentDescription(LABEL).performMouseInput { exit(Offset(-50f, -50f)) }
        mainClock.advanceTimeByFrame()
        assertEquals(false, hintShown(), "leaving the control hides the hint")
    }

    @Test
    fun aHeldFingerShowsTheLabelWithoutPressingTheControl() = runComposeUiTest {
        var clicks = 0
        control(onClick = { clicks++ })
        onNodeWithContentDescription(LABEL).performTouchInput { longClick() }
        mainClock.advanceTimeByFrame()
        onNodeWithText(LABEL).assertExists()
        assertEquals(0, clicks, "asking what a control does must not do it")
        mainClock.advanceTimeBy(HINT_LINGER_MS + 100)
        assertEquals(false, hintShown(), "the hint goes after it has been read")

        onNodeWithContentDescription(LABEL).performTouchInput { click() }
        mainClock.advanceTimeByFrame()
        assertEquals(1, clicks, "a tap still presses it")
        assertEquals(false, hintShown())
    }

    @Test
    fun aControlWithItsOwnLongPressKeepsItAndShowsNoHint() = runComposeUiTest {
        var longs = 0
        var clicks = 0
        control(onClick = { clicks++ }, onLongClick = { longs++ })
        onNodeWithContentDescription(LABEL).performTouchInput { longClick() }
        mainClock.advanceTimeByFrame()
        assertEquals(1, longs)
        assertEquals(0, clicks)
        assertEquals(false, hintShown())
    }
}
