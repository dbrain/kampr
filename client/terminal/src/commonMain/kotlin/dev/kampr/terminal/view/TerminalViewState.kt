package dev.kampr.terminal.view

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.geometry.Offset
import dev.kampr.terminal.render.Selection
import dev.kampr.terminal.render.Target
import kotlin.math.abs

// Probe #60: re-shaping at every intermediate zoom collapses the run cache to ~51% and drops 8.5%
// of frames at 200x50, so a pinch scales a layer and only the settled zoom re-shapes. Pan is a
// plain origin offset with no re-shaping, so it is applied straight to the committed state —
// putting it in the layer too would translate already-painted rows and leave the newly revealed
// ones blank until the finger lifts.
@Stable
class TerminalViewState {
    var zoom by mutableFloatStateOf(0f)
        private set
    var panX by mutableFloatStateOf(0f)
    var scrollY by mutableFloatStateOf(0f)

    var layerScale by mutableFloatStateOf(1f)
        private set
    var layerTx by mutableFloatStateOf(0f)
        private set
    var layerTy by mutableFloatStateOf(0f)
        private set

    var pinching by mutableStateOf(false)
        private set
    var sheetOpen by mutableStateOf(false)

    // Whether a controller is being held open on this pane by `pane.size`. Session-local and never
    // persisted, unlike `remembered`: a held pane is one the desk cannot reshape (#18) and renders
    // wrong at (#298), so it has to be asked for again every time rather than remembered into.
    var sizeHeld by mutableStateOf(false)
    var selection by mutableStateOf<Selection?>(null)
    var blockSelect by mutableStateOf(false)
    var target by mutableStateOf<Target?>(null)

    // Where a right-click landed, and the whole of the grid's context menu state. Session-local
    // and never persisted: a menu is a gesture that has not finished yet.
    var menuAt by mutableStateOf<Offset?>(null)
    var followCursor by mutableStateOf(true)
    var remembered by mutableStateOf(true)

    var velocityX = 0f
    var velocityY = 0f
    var minPanX = 0f
    var maxScroll = 0f

    // Where the surface rests while it follows, and nothing else. It was also the *floor of the
    // drag*, and that is the half that had to go: on a grid taller than the viewport with the
    // caret above the bottom of it, clamping a hand at the floor put the last rows of the pane
    // out of reach altogether — the wheel stopped early and every caret move re-clamped whatever
    // the hand had won back. The report was "keeps bouncing around and landing back where i last
    // typed instead of the bottom of the screen". A hand may travel the whole surface now; the
    // band governs the surface only while it is following, which is the thing the band was for.
    var band = CaretBand(0f, 0f)

    // Whether the viewport is riding the live edge rather than parked somewhere a reader put it.
    // A hand is the only thing that sets it: the surface itself rests anywhere in the caret band
    // while it follows, so its position is no longer the answer to "is this a reader's viewport" —
    // where the reader's hand last let go is.
    var following by mutableStateOf(true)
        private set

    var flings by mutableIntStateOf(0)
        private set

    fun fling() {
        flings++
    }

    val displayZoom: Float get() = zoom * layerScale

    var chosen = false
        private set

    // The opening scroll is re-derived as history, prefs and the measured insets land — all of
    // which arrive after the first paint. Once the reader has moved the viewport themselves that
    // re-derivation is a yank, and `chosen` does not cover it: it means "picked a zoom", which a
    // reader who has only ever dragged never does.
    var scrolled = false
        private set

    // The live edge is not scroll zero. The floor holds the surface off the bottom of the grid by
    // however far the caret sits above it, so a hand that lets go at the edge of a shell pane lets
    // go at a positive scrollY — which is what every "did they land on the edge" test here has to
    // ask about, and the reason they all ask it through this.
    //
    // *At* the floor rather than at-or-below it, now that below it is somewhere a hand can be. A
    // reader who dragged past the live edge to read the tail of the grid is parked there on
    // purpose, and reading that as "back on the live edge" is what bounced them straight back to
    // the caret. Typing is what re-arms it from anywhere — `followAgain`.
    private val atLiveEdge: Boolean get() = abs(scrollY - band.floor) <= FOLLOW_SLACK

    // A change of cell size invalidates every distance across this surface, and the opening one is
    // measured twice: the first composition lays out at the placeholder 13sp and the font re-probe
    // corrects it a few frames later — on a 90-row pane, from 18 pixels a row to 10 (#380). Nothing about
    // the scroll taken against the first is worth carrying into the second, and the floor is not a
    // linear function of the cell, so a follower is placed again rather than scaled or clamped. A
    // reader who has taken the viewport keeps what they took.
    fun placeOnFloor(floor: Float) {
        if (following) scrollY = floor
    }

    // Typing is a request to be shown what you typed, and it is the only way back to the live edge
    // from a viewport a hand has taken: a drag no longer lands on the floor by being clamped there,
    // so nothing else can put a reader back on it exactly. Every byte this client sends the pane
    // arms it, which is the same bargain the horizontal axis makes in `chaseCursor`.
    fun followAgain() {
        following = true
    }

    // Rows leaving the live grid extend the surface *below* a reader parked in history, and
    // scrollY is measured from that bottom — so standing still means moving with it. A reader
    // pinned to the bottom is pinned deliberately and must not be carried off it.
    //
    // Asked of `following` rather than of the position, because a surface that is following rests
    // anywhere in the caret band and a positive scrollY is no longer evidence of a hand. Reading
    // it as one carried a follower off the output it was following, which is #175 again by a
    // different route (#380).
    fun carryHistory(rowsAdded: Int, cellHeight: Float) {
        if (rowsAdded == 0 || following) return
        scrollY = (scrollY + rowsAdded * cellHeight).coerceAtLeast(0f)
    }

    // Both axes take the delta with the same sign: the surface goes where the finger goes. The
    // vertical one was subtracted, which made dragging down mean "newer" on a surface whose
    // horizontal drag already meant "the sheet moves with me".
    fun scrollBy(dx: Float, dy: Float) {
        scrolled = true
        if (dx != 0f) pannedAway = true
        panX = (panX + dx).coerceIn(minPanX, 0f)
        scrollY = (scrollY + dy).coerceIn(0f, maxScroll)
        following = atLiveEdge
    }

    // The horizontal half of `following`, and it was missing. `followCursorPan` puts the caret a
    // margin in from the left, and a caret inside that margin lands the surface on `panX = 0` —
    // the start of the line — so a reader who had dragged along a long line was put back there by
    // the next frame that moved the caret, and a second drag started over. A hand on the axis owns
    // it; the caret takes it back by arriving on screen itself, which is the bargain `following`
    // already makes with the live edge.
    var pannedAway = false
        private set

    fun chaseCursor(wanted: Float?) {
        panX = panX.coerceIn(minPanX, 0f)
        if (wanted == null) pannedAway = false else if (!pannedAway) panX = wanted
    }

    // Pan and scroll are distances across the surface, not across the viewport, so a change of
    // cell size has to carry them or the viewport lands on a different row than the one being read.
    private fun rescale(target: Float) {
        val applied = if (zoom > 0f) target / zoom else 1f
        panX *= applied
        scrollY *= applied
        zoom = target
    }

    // The default is re-derived until the operator picks a zoom of their own: history and the real
    // geometry both arrive after the first paint, and a zoom taken before them is wrong for good.
    fun adoptDefault(value: Float) {
        if (!chosen) rescale(value)
    }

    fun setZoom(value: Float, presets: ZoomPresets) {
        chosen = true
        rescale(value.coerceIn(presets.minimum, presets.maximum))
    }

    fun pinch(centroidX: Float, centroidY: Float, panDx: Float, panDy: Float, scale: Float) {
        pinching = true
        layerScale *= scale
        layerTx = centroidX + (layerTx - centroidX) * scale + panDx
        layerTy = centroidY + (layerTy - centroidY) * scale + panDy
    }

    // Folding the layer back in: a surface point v lands at v*S + T, and origins are
    // originX = panX and originY = contentBottom - surfaceHeight + scrollY, so
    // panX' = panX*S + Tx and scrollY' = scrollY*S + Ty + contentBottom*(S - 1).
    fun settle(presets: ZoomPresets, contentBottom: Float) {
        if (!pinching) return
        pinching = false
        val scale = layerScale
        val tx = layerTx
        val ty = layerTy
        layerScale = 1f
        layerTx = 0f
        layerTy = 0f
        chosen = true
        val target = (zoom * scale).coerceIn(presets.minimum, presets.maximum)
        val applied = if (zoom > 0f) target / zoom else 1f
        panX = panX * applied + tx
        scrollY = (scrollY * applied + ty + contentBottom * (applied - 1f))
            .coerceIn(0f, maxScroll.coerceAtLeast(0f))
        following = atLiveEdge
        zoom = target
    }
}

// A drag that lands on the floor lands exactly on it, because that is where it was clamped.
private const val FOLLOW_SLACK = 0.5f
