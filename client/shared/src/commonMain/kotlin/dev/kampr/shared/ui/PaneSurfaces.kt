package dev.kampr.shared.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.ProvidableCompositionLocal
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import dev.kampr.shared.model.PaneState
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
}

private object NoPaneIo : PaneIo {
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String): PanePrefs = PanePrefs()
}

val LocalPaneIo: ProvidableCompositionLocal<PaneIo> = staticCompositionLocalOf { NoPaneIo }
