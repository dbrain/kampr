package dev.kampr.terminal

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.PixelMap
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.test.runDesktopComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.terminal.render.GridPoint
import dev.kampr.terminal.render.Selection
import dev.kampr.terminal.render.Target
import dev.kampr.terminal.render.TargetKind
import dev.kampr.terminal.view.TargetStrip
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val SHORT = "https://a.b/c"

// One host, one very long path, and no punctuation the detector would trim. Ellipsised at any
// width this fits in a card, `https://herdr.dev.evil.example/…` and `https://herdr.dev/…` read as
// the same address — which is the whole reason the words are never allowed to end in one.
private const val LONG =
    "https://herdr.dev.evil.example/a/very/long/path/that/keeps/going/and/going/" +
        "and/going/until/it/is/quite/certainly/wider/than/any/card/this/app/will/draw"

// The operator's report was that clicking a path put the answer at the bottom of the screen with
// nothing saying which of the paths on screen it belonged to. Two halves: put the offer where the
// click was, and mark the cells the click hit.
@OptIn(ExperimentalTestApi::class)
class ShowingWhatWasClickedTest {
    @Test
    fun onADeskTheOfferLandsBesideTheClickRatherThanAtTheFootOfThePane() = runComposeUiTest {
        val session = PaneSession(Phone.PANE)
        deskTerminal(paneShowing(SHOWN), session, Route(words("hello")))
        val at = tapCell(session, 0, SHOWN.indexOf(NOTES) + 2)
        val clicked = with(density) { at.y.toDp() }
        val open = onNodeWithContentDescription("Open $NOTES").getUnclippedBoundsInRoot()
        assertTrue(
            open.top > clicked - 24.dp && open.top < clicked + 160.dp,
            "the click was at $clicked and the offer is at ${open.top}, ${DESK_HEIGHT} of pane tall",
        )
    }

    // A phone keeps the strip. Anchoring an affordance at a touch puts it under the thumb that
    // made the touch, and the bottom of a 914 dp screen is a thumb's own travel rather than a
    // mouse's — the decision is the form factor, not the platform.
    @Test
    fun onAPhoneTheOfferStaysOnTheStripAlongTheBottom() = runComposeUiTest {
        val session = PaneSession(Phone.PANE)
        gridTerminal(paneShowing(SHOWN), session, Route(words("hello")))
        val at = tapCell(session, 0, SHOWN.indexOf(NOTES) + 2)
        val clicked = with(density) { at.y.toDp() }
        val open = onNodeWithContentDescription("Open $NOTES").getUnclippedBoundsInRoot()
        assertTrue(
            open.top > clicked + 400.dp,
            "the strip moved off the bottom of the phone: click at $clicked, offer at ${open.top}",
        )
    }

    @Test
    fun theCellsThatWereHitAreTheOnesTheGridWashes() = runComposeUiTest {
        val session = PaneSession(Phone.PANE)
        gridTerminal(paneShowing(SHOWN), session, Route(words("hello")))
        val from = SHOWN.indexOf(NOTES)
        tapCell(session, 0, from + 2)
        assertEquals(
            Selection(GridPoint(0, from), GridPoint(0, from + NOTES.length - 1)),
            session.view.targetSpan,
        )
    }

    @Test
    fun nothingIsWashedOnceTheOfferIsGone() = runComposeUiTest {
        val session = PaneSession(Phone.PANE)
        gridTerminal(paneShowing(SHOWN), session, Route(words("hello")))
        tapCell(session, 0, SHOWN.indexOf(NOTES) + 2)
        tapCell(session, 0, 0)
        assertEquals(null, session.view.targetSpan)
    }

    // The span reaching `TerminalViewState` is not the same claim as a wash reaching the screen:
    // the grid paints into a `Canvas` and nothing in the semantics tree can see what came out of
    // it. So this reads the pixels — the cells the path was written in have to change when it is
    // hit, and a blank row six lines down has to be exactly as it was.
    @Test
    fun theWashIsPaintedOnTheCellsAndNowhereElse() = runDesktopComposeUiTest(411, 914) {
        val session = PaneSession(Phone.PANE)
        gridTerminal(paneShowing(SHOWN), session, Route(words("hello")))
        val from = SHOWN.indexOf(NOTES)
        val hit = band(session, 0, from, from + NOTES.length - 1)
        val blank = band(session, 6, from, from + NOTES.length - 1)
        val before = onRoot().captureToImage().toPixelMap()
        tapCell(session, 0, from + 2)
        val after = onRoot().captureToImage().toPixelMap()
        assertTrue(hit.last <= before.height && blank.last <= before.height, "the bands are off the capture")
        assertTrue(
            changed(before, after, hit) > 0.9f,
            "only ${changed(before, after, hit)} of the path's own cells changed when it was hit",
        )
        assertEquals(
            0f,
            changed(before, after, blank),
            "the wash reached a blank row six lines below the path",
        )
    }

    // Both strips at the same width in the same composition, so the comparison is a layout fact
    // rather than a line height this runner's monospace happened to resolve.
    @Test
    fun aLongAddressIsShownWholeRatherThanEllipsisedToASecondAddress() = runComposeUiTest {
        setContent {
            CompositionLocalProvider(LocalTokens provides Phone.tokens()) {
                Column(Modifier.size(411.dp, 914.dp)) {
                    TargetStrip(Target(SHORT, TargetKind.Url), {}, {}, Modifier)
                    TargetStrip(Target(LONG, TargetKind.Url), {}, {}, Modifier)
                }
            }
        }
        val shortBox = onNodeWithText(SHORT, useUnmergedTree = true).getUnclippedBoundsInRoot()
        val longBox = onNodeWithText(LONG, useUnmergedTree = true).getUnclippedBoundsInRoot()
        val short = shortBox.bottom - shortBox.top
        val long = longBox.bottom - longBox.top
        assertTrue(
            long > short * 2,
            "a ${LONG.length}-character address is laid out $long tall against $short for one of " +
                "${SHORT.length}, so it is being cut off rather than wrapped",
        )
    }
}

private class Band(val x0: Int, val y0: Int, val x1: Int, val y1: Int) {
    val last: Int get() = y1
}

@OptIn(ExperimentalTestApi::class)
private fun band(session: PaneSession, row: Int, from: Int, to: Int): Band {
    val grid = session.grid
    return Band(
        (grid.originX + from * grid.cellWidth).toInt() + 1,
        (grid.originY + row * grid.cellHeight).toInt() + 1,
        (grid.originX + (to + 1) * grid.cellWidth).toInt() - 1,
        (grid.originY + (row + 1) * grid.cellHeight).toInt() - 1,
    )
}

private fun changed(before: PixelMap, after: PixelMap, band: Band): Float {
    var moved = 0
    var total = 0
    for (y in band.y0 until band.y1) {
        for (x in band.x0 until band.x1) {
            total++
            if (before[x, y] != after[x, y]) moved++
        }
    }
    return if (total == 0) 0f else moved.toFloat() / total
}
