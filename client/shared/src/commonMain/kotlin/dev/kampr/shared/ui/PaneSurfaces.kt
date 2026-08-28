package dev.kampr.shared.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.ProvidableCompositionLocal
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.Dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.PanePrefs

interface PaneSurfaces {
    @Composable
    fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier)

    @Composable
    fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier)

    @Composable
    fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier)

    // The header carries the zoom control, but only the terminal surface knows what the zoom is.
    @Composable
    fun Zoom(pane: PaneState, modifier: Modifier) = Unit
}

// A surface is handed a PaneState, not the connection, so this is how a renderer answers back:
// keystrokes out, and the node's per-device prefs in. Zoom is stored server-side against the
// device, so it follows the operator between browsers.
interface PaneIo {
    fun send(msg: ClientMsg)
    fun prefs(paneId: String): PanePrefs

    // A surface that guards input has to know what is on the other end of it: a pane running an
    // agent is being typed *at*, not driven, and `rm -rf` in a prompt box is prose.
    fun info(paneId: String): PaneInfo? = null

    val readOnly: Boolean get() = false

    // A surface may need to hand the pane over to the other one — a harness with no journal
    // adapter has no conversation to show, and offers the terminal instead of an error.
    fun show(view: PaneView) = Unit

    // Told when a pane is being held at a size, because the status strip stands there saying "no
    // lease held — desktop shape untouched" and that sentence is false exactly while one is. A
    // held pane overrides whoever is at the desk (#18) and leaves their screen wrong (#298), so it
    // is the one state the operator most needs said out loud rather than assumed away.
    fun holding(paneId: String, held: Boolean) = Unit

    // The bytes behind a transcript attachment, on demand and over HTTP rather than over the
    // socket: the socket is carrying live terminal frames and a screenshot on it head-of-lines
    // every pane for seconds on a phone link.
    suspend fun attachment(paneId: String, id: String): AttachmentBytes =
        AttachmentBytes.Failed("This device has no node to fetch it from.")
}

private object NoPaneIo : PaneIo {
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String): PanePrefs = PanePrefs()
}

val LocalPaneIo: ProvidableCompositionLocal<PaneIo> = staticCompositionLocalOf { NoPaneIo }

// Chrome floats over the terminal surface rather than sitting above it: the paint fills the
// whole cell, the scrollable content insets under whatever is drawn on top. A surface guesses
// the inset from its own size unless something above it knows better — a mosaic cell's header
// is a third of a screen header's, and a cell full of blank rows is the bug that follows from
// guessing.
@Immutable
data class PaneChrome(val top: Dp)

val LocalPaneChrome: ProvidableCompositionLocal<PaneChrome?> = staticCompositionLocalOf { null }
