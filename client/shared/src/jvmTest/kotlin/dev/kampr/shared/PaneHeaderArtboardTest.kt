package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.ui.ConnectPanel
import dev.kampr.shared.ui.LocalManage
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.wire.PaneInfo
import java.io.File
import kotlin.test.Test
import kotlin.test.assertTrue

private val OUT = File("build/artboards")

private const val PANE_ID = "01JNODE/w1:p1"

private val INFO = PaneInfo(
    id = PANE_ID,
    nodeId = "01JNODE",
    workspace = "kampr",
    cwd = "/home/dbrain/dev/kampr",
    agent = "claude",
    agentStatus = "blocked",
    cols = 94,
    rows = 40,
    hasConversation = true,
)



// The two defects with something to look at, at the density they were reported on: 1080x2400 is
// 360 dp at 3x, which is where the header ran out of room.
class PaneHeaderArtboardTest {
    private fun board(name: String, width: Int, height: Int, density: Float, content: @Composable () -> Unit) {
        val image = render(
            width.dp, height.dp, themeOf("soft"), TypeScale.Phone, File(OUT, "$name.png"),
            density = Density(density),
        ) {
            CompositionLocalProvider(LocalSafeArea provides BARS, LocalManage provides AllowManage) { content() }
        }
        assertTrue(image.width > 0 && image.height > 0)
    }

    private fun pane(readOnly: Boolean = false, stale: Boolean = false, landscape: Boolean = false):
        @Composable () -> Unit = {
        PaneScreenMobile(
            pane = PaneState(PANE_ID, StyleTable()).also { it.stale = stale },
            info = INFO,
            view = PaneView.Terminal,
            surfaces = BlankSurfaces,
            landscape = landscape,
            readOnly = readOnly,
            onBack = {},
            onView = {},
            modifier = Modifier.fillMaxSize(),
        )
    }

    @Test
    fun thePaneHeaderRendersAtEveryDensityTheReportCovers() {
        board("pane-header-480dpi", 360, 800, 3f, pane())
        board("pane-header-480dpi-crowded", 360, 800, 3f, pane(readOnly = true, stale = true))
        board("pane-header-420dpi", 411, 914, 2.625f, pane())
        board("pane-header-480dpi-landscape", 800, 360, 3f, pane(landscape = true))
    }

    @Test
    fun theConnectPanelRenders() {
        board("connect-panel-480dpi", 360, 800, 3f) {
            ConnectPanel(Endpoint("http://192.168.1.24:8790"), null, {}, offeredCode = "2KQK-RB5Y")
        }
    }
}
