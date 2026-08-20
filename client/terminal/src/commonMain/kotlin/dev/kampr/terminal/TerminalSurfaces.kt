package dev.kampr.terminal

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.onSizeChanged
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.ui.FallbackSurfaces
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.terminal.input.InputSink
import dev.kampr.terminal.input.PaneKeyRow
import dev.kampr.terminal.view.TerminalView
import dev.kampr.terminal.view.ZoomButton

class TerminalSurfaces(private val conversation: PaneSurfaces = FallbackSurfaces) : PaneSurfaces {
    private val sessions = PaneSessions()

    @Composable
    override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) {
        TerminalView(pane, sessions[pane.id], LocalPaneIo.current, modifier)
    }

    @Composable
    override fun Zoom(pane: PaneState, modifier: Modifier) {
        ZoomButton(sessions[pane.id], modifier)
    }

    @Composable
    override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) {
        conversation.Conversation(pane, info, modifier)
    }

    @Composable
    override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) {
        val io = LocalPaneIo.current
        val session = sessions[pane.id]
        val sink = remember(pane.id, io, session) { InputSink(pane.id, io, session.latches) }
        // The terminal insets its scrollable content by whatever the key row actually occupies,
        // including the keyboard it is docked above, so the pinned last row settles clear of it.
        DisposableEffect(session) { onDispose { session.keyRowHeight = 0f } }
        PaneKeyRow(
            session = session,
            sink = sink,
            compact = compact,
            enabled = !io.readOnly,
            modifier = modifier.onSizeChanged { session.keyRowHeight = it.height.toFloat() },
        )
    }
}
