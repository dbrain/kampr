package dev.kampr.terminal

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.terminal.input.InputSink
import dev.kampr.terminal.input.PaneKeyRow
import dev.kampr.terminal.render.GridPoint
import dev.kampr.terminal.render.Selection
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotEquals
import kotlin.test.assertTrue

@OptIn(ExperimentalTestApi::class)
private fun keyRow(enabled: Boolean, body: ComposeUiTest.() -> Unit) =
    runComposeUiTest {
        val session = PaneSession(Phone.PANE)
        val sink = InputSink(Phone.PANE, HushIo, session.latches, null)
        setContent {
            CompositionLocalProvider(
                LocalTokens provides Phone.tokens(),
                LocalSafeArea provides Phone.BARS,
            ) {
                Box(Modifier.size(411.dp, 914.dp)) {
                    PaneKeyRow(session, sink, compact = false, enabled = enabled)
                }
            }
        }
        body()
    }

// Two surfaces, two answers, and neither of them was ever read back. The caps are controls driven
// by a raw tap detector, so they get nothing from the cursor `Modifier.action` already chains and
// hovered as a plain arrow on a desk. The grid is not a control at all: it is the one surface here
// a mouse drags a selection out of, so it is the one that keeps the I-beam.
//
// Both were reported untestable. They are not: `PointerHoverIconModifierElement` names its icon and
// its `overrideDescendants` in its own `toString`, and applying Compose's resolution rule to a
// node's modifier chain is what `HoverChain` does.
@OptIn(ExperimentalTestApi::class)
class PaneHoverCursorTest {
    @Test
    fun everyKeyRowCapHoversAsAHand() = keyRow(enabled = true) {
        val caps = onAllNodes(hasClickAction(), useUnmergedTree = true).fetchSemanticsNodes()
        assertTrue(caps.size >= 10, "the key row painted ${caps.size} caps")
        for (cap in caps) assertEquals(HAND, cap.cursor(), "a cap hovers as ${cap.cursor()}")
    }

    // A read-only device is offered no press at all, and a cap that refuses one is not a hand. The
    // plain arrow rather than nothing: leaving the cap alone hands the cursor back to whatever is
    // underneath, and on a bar of dead keys that is not an honest answer either.
    @Test
    fun aKeyRowOnAReadOnlyDeviceHoversAsNothingYouCanPress() = keyRow(enabled = false) {
        val cap = onNodeWithContentDescription(
            "Escape key, unavailable on a read-only device",
            useUnmergedTree = true,
        ).fetchSemanticsNode()
        assertEquals(ARROW, cap.cursor())
        assertNotEquals(HAND, cap.cursor(), "a cap nobody may press offered a hand")
    }

    @Test
    fun theGridHoversAsTextWhateverIsGoingOnOverIt() = runComposeUiTest {
        val pane = Phone.shell()
        val session = PaneSession(Phone.PANE)
        phoneTerminal(pane, session)

        fun grid() = onNodeWithContentDescription("Terminal grid", substring = true, useUnmergedTree = true)
            .fetchSemanticsNode()

        assertEquals(TEXT, grid().cursor(), "a bare grid hovers as ${grid().cursor()}")

        session.view.selection = Selection(GridPoint(0, 0), GridPoint(0, 10))
        waitForIdle()
        assertEquals(TEXT, grid().cursor(), "a grid with a selection standing hovers as ${grid().cursor()}")

        session.view.menuAt = Offset(40f, 40f)
        waitForIdle()
        assertEquals(TEXT, grid().cursor(), "a grid under its own menu hovers as ${grid().cursor()}")
        assertNotEquals(HAND, grid().cursor(), "the grid is a text surface, not a control")
    }
}

