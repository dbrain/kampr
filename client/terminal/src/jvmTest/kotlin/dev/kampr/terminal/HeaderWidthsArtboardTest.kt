package dev.kampr.terminal

import androidx.compose.foundation.layout.Box
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalManage
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.ManageIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.terminal.view.ZoomButton
import java.io.File
import kotlin.test.Test

private const val HEADER_PANE = "01JKAMPRNODE0000000000000/w3:p1"

private fun headerInfo(status: String) = PaneInfo(
    id = HEADER_PANE, nodeId = "01JKAMPRNODE0000000000000", workspace = "kampr", tab = "1",
    cwd = "~/dev/kampr", agent = "claude", agentStatus = status, cols = 94, rows = 40,
    hasConversation = true, label = "Wire the hooks socket",
)

private class HeaderIo(val info: PaneInfo) : PaneIo {
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String) = PanePrefs()
    override fun info(paneId: String) = info
}

private object HeaderManage : ManageIo {
    override val enabled = true
    override fun openNew(paneId: String?) = Unit
    override fun openActions(paneId: String) = Unit
}

private class HeaderZoom(private val session: PaneSession) : PaneSurfaces {
    @Composable override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = Unit

    @Composable override fun Zoom(pane: PaneState, modifier: Modifier) = ZoomButton(session, modifier)
}

private class HeaderLook(val name: String, val status: String, val quiet: Boolean = false, val stale: Boolean = false, val unsent: Int = 0, val readOnly: Boolean = false)

private val HEADER_LOOKS = listOf(
    HeaderLook("idle", "idle"),
    HeaderLook("working", "working"),
    HeaderLook("blocked", "blocked"),
    HeaderLook("quiet-working", "working", quiet = true),
    HeaderLook("everything", "blocked", stale = true, unsent = 3, readOnly = true),
)

private class HeaderScreen(val name: String, val width: Dp, val landscape: Boolean = false)

private val HEADER_SCREENS = listOf(
    HeaderScreen("360", 360.dp),
    HeaderScreen("393", 393.dp),
    HeaderScreen("411", 411.dp),
    HeaderScreen("448", 448.dp),
    HeaderScreen("landscape-780", 780.dp, landscape = true),
)

private fun headerPane(look: HeaderLook) = PaneState(HEADER_PANE, StyleTable()).also { pane ->
    pane.applyReset(ServerMsg.GridReset(HEADER_PANE, 4, 1, emptyList(), Cursor(0, 0, true), links = emptyList()))
    if (look.quiet) pane.noteStreamStopped()
    pane.stale = look.stale
    repeat(look.unsent) { pane.noteUndelivered() }
}

// Cropped to the chrome, one board per phone width and status mix, so the header can be compared
// across the screens it has to fit rather than on the one it was drawn for.
class HeaderWidthsArtboardTest {
    @Test
    fun thePhoneHeaderOnEveryWidth() {
        for (screen in HEADER_SCREENS) {
            for (look in HEADER_LOOKS) {
                val info = headerInfo(look.status)
                renderArtboard(
                    screen.width,
                    if (screen.landscape) 70.dp else 110.dp,
                    SoftTheme,
                    TypeScale.Phone,
                    File(File(OUT, "header-widths"), "${screen.name}-${look.name}.png"),
                    density = Density(2.5f),
                ) {
                    CompositionLocalProvider(LocalPaneIo provides HeaderIo(info), LocalManage provides HeaderManage) {
                        PaneScreenMobile(headerPane(look), info, PaneView.Terminal, HeaderZoom(PaneSession(HEADER_PANE)), screen.landscape, look.readOnly, onBack = {}, onView = {})
                    }
                }
            }
        }
    }
}
