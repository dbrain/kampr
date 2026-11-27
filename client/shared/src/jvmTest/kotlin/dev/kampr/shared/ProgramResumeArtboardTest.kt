package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.HerdPortrait
import dev.kampr.shared.ui.LocalManage
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.ManageIo
import dev.kampr.shared.ui.MenuAnchor
import dev.kampr.shared.ui.PaneScreenDesktop
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.ProgramsSheet
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ProgramRow
import dev.kampr.shared.wire.ResumeRecord
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import java.io.File
import kotlin.test.Test
import kotlin.test.assertTrue

private val OUT = File("build/artboards")

private const val BOX = "01JBOX"

private val HERD: Herd = (
    Wire.decode(
        """{"t":"herd","nodes":[{"id":"$BOX","name":"comingclean","kind":"local","online":true,"native":true}],"panes":[
         {"id":"$BOX/n:a","node_id":"$BOX","provider":"native","label":"deploy","cwd":"/home/u/dev/site","agent_status":"idle",
          "program_error":true,"program_msg":"terraform apply exited 1"},
         {"id":"$BOX/n:b","node_id":"$BOX","provider":"native","label":"cargo","cwd":"/home/u/dev/kampr","agent_status":"working",
          "program_progress":62}]}""",
    ) as ServerMsg.Herd
    ).let { Herd(it.nodes, it.panes, known = true) }

private val RECORDS = mapOf(
    BOX to listOf(
        ResumeRecord("01k8/74877", "notes", listOf("/usr/bin/nvim", "-S", "/home/u/.local/state/nvim/notes.vim"), "/home/u/notes"),
        ResumeRecord("01k9/1102", null, listOf("sh", "-c", "tail -f log; \$(reboot)"), "/home/u"),
    ),
)

private object Asking : ManageIo {
    override val enabled = true
    override fun openNew(paneId: String?) = Unit
    override fun openActions(paneId: String) = Unit
    override fun openMenu(paneId: String, at: MenuAnchor?) = Unit
    override fun resumable(nodeId: String): List<ResumeRecord> = RECORDS[nodeId].orEmpty()
}

private object Unpainted : PaneSurfaces {
    @Composable override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = Box(modifier)

    @Composable override fun Zoom(pane: PaneState, modifier: Modifier) = Unit
}

private val ROWS = listOf(
    ProgramRow("", "working", app = "ci", progress = 62),
    ProgramRow("test", "working", app = "ci", title = "Tests", progress = 40),
    ProgramRow("lint", "done", app = "ci", title = "Lint"),
    ProgramRow("lint/slow", "error", app = "ci", title = "Slow rules", msg = "3 rules failed: see lint.log"),
    ProgramRow("deploy", "blocked", app = "ci", kind = "permission", title = "Deploy", msg = "approve the production deploy"),
)

private fun pane(error: Boolean = false) = PaneInfo(
    id = "$BOX/n:b",
    nodeId = BOX,
    provider = "native",
    label = "cargo",
    cwd = "/home/u/dev/kampr",
    agentStatus = if (error) "idle" else "working",
    programError = error,
    programMsg = "terraform apply exited 1".takeIf { error },
    programProgress = 62.takeIf { !error },
    program = ROWS.takeIf { !error },
)

// docs/24 and docs/25 drawn: the herd with a failed and a progressing row and a boot's records,
// the headers that mark them, and the records sheet.
class ProgramResumeArtboardTest {
    private fun board(name: String, width: Int, height: Int, content: @Composable () -> Unit) {
        val image = render(width.dp, height.dp, themeOf("soft"), TypeScale.Phone, File(OUT, "$name.png"), density = Density(2f)) {
            CompositionLocalProvider(LocalSafeArea provides BARS, LocalManage provides Asking) { content() }
        }
        assertTrue(image.width > 0 && image.height > 0)
    }

    @Test
    fun herd() = board("program-resume-herd", 411, 900) {
        HerdPortrait(HERD, ConnectionStatus.Live("full"), 0.0, 12.0, emptyList(), {}, null)
    }

    @Test
    fun portraitHeaders() = board("program-resume-header-portrait", 411, 260) {
        androidx.compose.foundation.layout.Column {
            Box(Modifier.fillMaxSize().weight(1f)) {
                PaneScreenMobile(PaneState(pane().id, StyleTable()), pane(), PaneView.Terminal, Unpainted, false, false, onBack = {}, onView = {})
            }
            Box(Modifier.fillMaxSize().weight(1f)) {
                PaneScreenMobile(PaneState(pane().id, StyleTable()), pane(error = true), PaneView.Terminal, Unpainted, false, false, onBack = {}, onView = {})
            }
        }
    }

    @Test
    fun desktopHeader() = board("program-resume-header-desktop", 1100, 160) {
        androidx.compose.foundation.layout.Column {
            Box(Modifier.fillMaxSize().weight(1f)) {
                PaneScreenDesktop(PaneState(pane().id, StyleTable()), pane(), PaneView.Terminal, Unpainted, false, onView = {})
            }
            Box(Modifier.fillMaxSize().weight(1f)) {
                PaneScreenDesktop(PaneState(pane().id, StyleTable()), pane(error = true), PaneView.Terminal, Unpainted, false, onView = {})
            }
        }
    }

    @Test
    fun sheet() = board("program-resume-sheet", 411, 600) {
        ProgramsSheet("cargo · bash", ROWS, Breakpoint.Portrait) {}
    }
}
