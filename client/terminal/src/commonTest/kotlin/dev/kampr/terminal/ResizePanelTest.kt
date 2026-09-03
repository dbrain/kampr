package dev.kampr.terminal

import dev.kampr.shared.wire.MIN_PANE_COLS
import dev.kampr.shared.wire.MIN_PANE_ROWS
import dev.kampr.terminal.view.PaneSizing
import dev.kampr.terminal.view.SIZE_PRESETS
import dev.kampr.terminal.view.fitIsUsable
import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

// The guard the whole feature is shaped around. A resize on a headless pane persists after the
// controller lets go (#219), so "fit this pane to my screen" from a phone would leave that pane at
// phone width for every other client, with nothing but another resize to undo it.
class ResizePanelTest {
    private fun sizing(viewCols: Int, viewRows: Int) =
        PaneSizing(cols = 94, rows = 40, viewCols = viewCols, viewRows = viewRows, held = false)

    @Test
    fun aViewTooNarrowToGiveAPaneIsRefused() {
        assertFalse(sizing(45, 20).fitIsUsable(), "a zoomed-in phone must not be offered its own size")
        assertFalse(sizing(MIN_PANE_COLS - 1, 40).fitIsUsable(), "one column under the floor is under it")
        assertFalse(sizing(120, MIN_PANE_ROWS - 1).fitIsUsable(), "rows count too")
    }

    @Test
    fun aDeskSizedViewIsOffered() {
        assertTrue(sizing(MIN_PANE_COLS, MIN_PANE_ROWS).fitIsUsable(), "the floor itself is allowed")
        assertTrue(sizing(292, 72).fitIsUsable(), "a real desk browser")
        // Measured at the base cell, so this is a window that really is 94 columns wide at
        // readable size — the pane is what is being sized, not the reading, and zooming out to
        // see a wide pane no longer moves this number at all.
        assertTrue(sizing(94, 40).fitIsUsable())
    }

    // Every quick preset has to clear the floor the node enforces, or a one-tap button would be a
    // one-tap `bad_request`.
    @Test
    fun everyQuickPresetIsOneTheNodeWillAccept() {
        for ((cols, rows) in SIZE_PRESETS) {
            assertTrue(
                cols >= MIN_PANE_COLS && rows >= MIN_PANE_ROWS,
                "$cols×$rows is below the ${MIN_PANE_COLS}×$MIN_PANE_ROWS the node refuses",
            )
        }
    }
}
