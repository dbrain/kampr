package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.KeyInjectionScope
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.test.withKeyDown
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.Herd
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.KField
import dev.kampr.shared.ui.PaletteHost
import dev.kampr.shared.ui.PaletteTarget
import dev.kampr.shared.ui.paletteItems
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private val KEY_HERD = Herd(
    nodes = listOf(NodeInfo("01JLAP", "comingclean", kind = "local"), NodeInfo("01JHUB", "giftofthemagi2")),
    panes = listOf(
        PaneInfo("01JLAP/w1:p1", "01JLAP", workspace = "kampr", cwd = "/home/dbrain/dev/kampr"),
        PaneInfo("01JLAP/w2:p1", "01JLAP", workspace = "notes", cwd = "/home/dbrain/notes"),
        PaneInfo("01JHUB/w1:p1", "01JHUB", workspace = "herdr", cwd = "/home/dbrain/dev/herdr"),
    ),
)

private const val FIELD = "Search panes and places"
private const val BEHIND = "something behind the palette"

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.host(picked: MutableList<PaletteTarget>) {
    setContent {
        CompositionLocalProvider(LocalTokens provides phoneTokens()) {
            PaletteHost({ paletteItems(KEY_HERD, mosaic = false, canCreate = false) }, { picked += it }, Modifier.size(800.dp, 600.dp)) {
                Box(Modifier.size(800.dp, 600.dp)) { KField(BEHIND, "", onText = {}) }
            }
        }
    }
    onNodeWithContentDescription(BEHIND).performClick()
    onNodeWithContentDescription(BEHIND).performKeyInput { ctrl(Key.K) }
    waitForIdle()
}

@OptIn(ExperimentalTestApi::class)
private fun KeyInjectionScope.ctrl(key: Key) = withKeyDown(Key.CtrlLeft) { pressKey(key) }

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.open(): Boolean = onAllNodesWithContentDescription(FIELD).fetchSemanticsNodes().isNotEmpty()

@OptIn(ExperimentalTestApi::class)
class PaletteKeysTest {
    @Test
    fun ctrlKFromAFocusedFieldOpensItAndADigitJumps() = runComposeUiTest {
        val picked = mutableListOf<PaletteTarget>()
        host(picked)
        assertTrue(open(), "ctrl+K did not open the palette from inside another text field")
        onNodeWithContentDescription(FIELD).performTextInput("magi")
        onNodeWithContentDescription(FIELD).performKeyInput { pressKey(Key.One) }
        assertEquals(listOf<PaletteTarget>(PaletteTarget.OpenPane("01JHUB/w1:p1")), picked)
        assertTrue(!open(), "the palette stayed up after a pick")
    }

    @Test
    fun anEmptyQueryNumbersTheSidebar() = runComposeUiTest {
        val picked = mutableListOf<PaletteTarget>()
        host(picked)
        onNodeWithContentDescription(FIELD).performKeyInput { pressKey(Key.Two) }
        assertEquals(listOf<PaletteTarget>(PaletteTarget.OpenPane("01JLAP/w2:p1")), picked)
    }

    @Test
    fun arrowsAndEnterPickAndEscapeLeaves() = runComposeUiTest {
        val picked = mutableListOf<PaletteTarget>()
        host(picked)
        onNodeWithContentDescription(FIELD).performKeyInput {
            pressKey(Key.DirectionDown)
            pressKey(Key.DirectionDown)
            pressKey(Key.Enter)
        }
        assertEquals(listOf<PaletteTarget>(PaletteTarget.OpenPane("01JHUB/w1:p1")), picked)

        onNodeWithContentDescription(BEHIND).performClick()
        onNodeWithContentDescription(BEHIND).performKeyInput { ctrl(Key.K) }
        waitForIdle()
        onNodeWithContentDescription(FIELD).performKeyInput { pressKey(Key.Escape) }
        assertTrue(!open(), "escape left the palette up")
        assertEquals(1, picked.size)
    }

    @Test
    fun aRowIsNamedByItsDigit() = runComposeUiTest {
        val picked = mutableListOf<PaletteTarget>()
        host(picked)
        onNodeWithContentDescription("3: ", substring = true).performClick()
        assertEquals(listOf<PaletteTarget>(PaletteTarget.OpenPane("01JHUB/w1:p1")), picked)
    }
}
