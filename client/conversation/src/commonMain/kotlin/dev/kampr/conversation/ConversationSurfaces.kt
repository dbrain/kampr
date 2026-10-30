package dev.kampr.conversation

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import dev.kampr.conversation.files.FilesHost
import dev.kampr.conversation.files.HeldBytes
import dev.kampr.conversation.files.LocalHeldBytes
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.ui.FallbackSurfaces
import dev.kampr.shared.ui.FilesNav
import dev.kampr.shared.ui.FilesPlace
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.wire.PaneInfo

class ConversationSurfaces(private val base: PaneSurfaces = FallbackSurfaces) : PaneSurfaces {
    private val held = HeldBytes()

    @Composable
    override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = base.Terminal(pane, info, modifier)

    @Composable
    override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = CompositionLocalProvider(LocalHeldBytes provides held) {
        PathLinks(pane.id, info?.cwd) { ConversationView(pane, info, modifier) }
    }

    @Composable
    override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = base.KeyRow(pane, compact, modifier)

    @Composable
    override fun Zoom(pane: PaneState, modifier: Modifier) = base.Zoom(pane, modifier)

    @Composable
    override fun Files(place: FilesPlace, nav: FilesNav, modifier: Modifier) = CompositionLocalProvider(LocalHeldBytes provides held) {
        FilesHost(place, nav, modifier)
    }
}
