package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Turn
import kotlin.test.Test
import kotlin.test.assertTrue

// Long enough to wrap on a desktop column, so its box is the frame's own width and can stand as
// the reference every other box is measured against.
private const val PROSE =
    "The width inference reads the pane's own wrap rather than the layout rect, because the rect " +
        "is the outer box and the column it keeps back belongs to the scrollbar, which is the " +
        "distinction that cost three afternoons and two probe rows before anybody wrote it down."

private const val SHORT_CODE = "val x = 1"

private fun fittedPane(): KamprStore {
    val store = KamprStore()
    store.accept(
        ServerMsg.Convo(
            pane = PANE_ID, cursor = "f-1", more = false,
            turns = listOf(
                proseTurn("f-1", "how wide is it?", role = "user"),
                Turn(
                    "f-2", "assistant", null,
                    listOf(
                        Block.Md(PROSE),
                        Block.Code("kotlin", SHORT_CODE),
                        // Last, and with nothing after it: a code or diff block that follows a
                        // call is that call's output and is folded into it.
                        Block.Tool("Bash", "ls", 3, "done"),
                    ),
                ),
            ),
        ),
    )
    return store
}

@Composable
private fun Wide(store: KamprStore) {
    CompositionLocalProvider(
        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
        LocalPaneIo provides RecordingIo,
    ) {
        Box(Modifier.size(DESKTOP.first, DESKTOP.second)) {
            ConversationView(store.pane(PANE_ID), demoInfo(status = "idle"), Modifier.fillMaxSize())
        }
    }
}

// The report, from a desk: *"we seem to stretch everything to full width for no great reason …
// tool calls/etc. expand the whole view"*.
//
// The frame is the operator's column and stays it. What sat inside the frame was every box
// `fillMaxWidth`, so a call reading four words and a code block holding one line were both drawn a
// metre wide with nothing in them, and the reader's eye had to cross the empty half of each to get
// to the next line.
//
// The mutation that must fail: put `fillMaxWidth` back on either card and its box is the prose's
// own width again.
@OptIn(ExperimentalTestApi::class)
class FitWidthTest {
    @Test
    fun aCardIsAsWideAsWhatIsInItAndTheProseStillTakesTheColumn() = runComposeUiTest {
        setContent { Wide(fittedPane()) }
        waitForIdle()

        // The reply's own head is the column: it fills the frame by rule and is the one row on the
        // screen whose width is the width every box under it *could* have taken. Measured against
        // the scene rather than against `DESKTOP`, because the headless window has a size of its
        // own and the point being made is a ratio.
        val scene = onRoot().fetchSemanticsNode().boundsInRoot
        val column = onNodeWithContentDescription("Put away the reply of", substring = true)
            .fetchSemanticsNode().boundsInRoot
        val prose = onNodeWithText(PROSE, substring = true).fetchSemanticsNode().boundsInRoot
        val call = onNodeWithContentDescription("Tool Bash, ls, 3 lines of output").fetchSemanticsNode().boundsInRoot
        val copy = onNodeWithContentDescription("Copy the kotlin block").fetchSemanticsNode().boundsInRoot

        assertTrue(
            column.width > scene.width * 0.8f,
            "the reply frame no longer takes the column it is read in: ${column.width}",
        )
        // Prose wraps to the frame it is given, so its longest line is most of that frame — the
        // reading measure is what the frame is for and it is not what changes here.
        assertTrue(
            prose.width > column.width / 2f,
            "the prose stopped filling the frame it wraps in: ${prose.width} of ${column.width}",
        )
        assertTrue(
            call.width < column.width / 2f,
            "a four-word tool call is still drawn the width of the frame: ${call.width} of ${column.width}",
        )
        // The code card carries no name of its own, so it is measured by where its copy control
        // ends up: on a card that fits one short line, that is nowhere near the far edge.
        assertTrue(
            copy.right < column.left + column.width / 2f,
            "a one-line code block is still drawn the width of the frame: ${copy.right} of ${column.right}",
        )
        assertTrue(
            call.left == column.left && copy.left > column.left,
            "a card that fits its content has left the column it is stacked in",
        )
    }
}
