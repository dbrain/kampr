package dev.kampr.shared.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.ProvidableCompositionLocal
import dev.kampr.shared.model.ConnectionStatus
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.Dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.SizeMode
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

    // ADR 0013's standing hold, asked for and let go of through the *session* rather than straight
    // off the view that wanted it — because a release is a resize. It puts the pane back to the
    // geometry it was found at, so one terminal view leaving the composition and another arriving
    // (which is the whole of a pane switch) wrote a geometry onto the pane being left and another
    // onto the pane being opened, and switching back wrote both again the other way round. See
    // `MatchHolds`, which is where the linger and the already-held rule live.
    //
    // The defaults are the wire, so a surface with no session behind it behaves exactly as the
    // view used to. `linger` is false where the *operator* said so — ticking the switch off is an
    // answer about this pane and is owed the pane back at once — and true where a view merely
    // ended, which is not an answer at all.
    fun claimMatch(paneId: String, cols: Int, rows: Int) =
        send(ClientMsg.Manage(ManageOp.PaneSize(paneId, cols, rows, SizeMode.Match)))

    fun releaseMatch(paneId: String, linger: Boolean = true) =
        send(ClientMsg.Manage(ManageOp.PaneSize(paneId, mode = SizeMode.Release)))

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
// True inside a mosaic cell, where a pane is one thumbnail among several rather than the thing
// being looked at. Two cells on a wide desktop each measure as a desk, and holding a pane at the
// size of a tile in a grid is exactly the write rule 3 forbids — see ADR 0013, whose whole gate is
// "is this window the terminal somebody is working in".
val LocalMosaicCell = staticCompositionLocalOf { false }

@Immutable
data class PaneChrome(val top: Dp)

val LocalPaneChrome: ProvidableCompositionLocal<PaneChrome?> = staticCompositionLocalOf { null }

// The socket's own state, where a pane surface can read it. The conversation needs it and the grid
// does not: a grid that has stopped arriving is visible as a grid that has stopped, while a
// transcript drawn from memory looks exactly like a transcript that is current.
val LocalConnectionStatus: ProvidableCompositionLocal<ConnectionStatus> =
    staticCompositionLocalOf { ConnectionStatus.Idle }
