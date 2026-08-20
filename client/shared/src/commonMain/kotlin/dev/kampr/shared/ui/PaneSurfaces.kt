package dev.kampr.shared.ui

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.wire.PaneInfo

interface PaneSurfaces {
    @Composable
    fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier)

    @Composable
    fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier)

    @Composable
    fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier)
}
