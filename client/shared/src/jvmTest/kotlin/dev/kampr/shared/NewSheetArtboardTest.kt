package dev.kampr.shared

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.Herd
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.HerdPortrait
import dev.kampr.shared.ui.LocalManage
import dev.kampr.shared.ui.ManageIo
import dev.kampr.shared.ui.NewSheet
import dev.kampr.shared.ui.PaneActionsSheet
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.SessionInfo
import java.io.File
import kotlin.test.Test
import kotlin.test.assertTrue
import kotlin.test.fail

private val OUT = File("build/artboards")

// The 20 Herdr detects on the machine this was probed on (#48). They arrive over `caps`, and the
// sheet must render whatever it is handed rather than a list of its own.
private val KINDS = listOf(
    "aider", "amp", "claude", "cline", "codex", "continue", "copilot", "cursor", "gemini", "goose",
    "grok", "kilo", "opencode", "plandex", "qwen", "roo", "sst", "sweep", "windsurf", "zed",
).sorted()

private val NODE = NodeInfo(id = "01JNODE", name = "comingclean", kind = "local")

private val PANE = PaneInfo(
    id = "01JNODE/w3:p2",
    nodeId = "01JNODE",
    workspaceId = "01JNODE/w3",
    tabId = "01JNODE/w3:t1",
    workspace = "kampr",
    tab = "1",
    cwd = "/home/dbrain/dev/kampr",
    agent = "claude",
    agentStatus = "blocked",
    cols = 74,
    rows = 30,
)

private val CAPS = ServerMsg.NodeCaps(
    node = "01JNODE",
    agentKinds = KINDS,
    sessions = listOf(SessionInfo("default", true), SessionInfo("agents", false)),
)

class NewSheetArtboardTest {
    private fun sheet(): @Composable (Breakpoint) -> Unit = { breakpoint ->
        NewSheet(
            breakpoint = breakpoint,
            node = NODE,
            pane = PANE,
            nodes = listOf(NODE, NodeInfo("01JPI", "sungrow-pi"), NodeInfo("01JNAS", "nas")),
            caps = CAPS,
            outcome = null,
            onManage = {},
            onNode = {},
            onNodePicker = {},
            onDismiss = {},
        )
    }

    @Test
    fun theSheetRendersAtEveryBreakpoint() {
        for ((name, size) in listOf("portrait" to PORTRAIT, "landscape" to LANDSCAPE, "desktop" to DESKTOP)) {
            val breakpoint = when (name) {
                "portrait" -> Breakpoint.Portrait
                "landscape" -> Breakpoint.Landscape
                else -> Breakpoint.Desktop
            }
            val scale = if (breakpoint == Breakpoint.Desktop) TypeScale.Desk else TypeScale.Phone
            val image = render(size.first, size.second, themeOf("soft"), scale, File(OUT, "new-sheet-$name.png")) {
                sheet()(breakpoint)
            }
            assertTrue(image.width > 0 && image.height > 0)
        }
    }

    @Test
    fun theSheetRendersInASecondThemeAndOnALightGround() {
        render(PORTRAIT.first, PORTRAIT.second, themeOf("phosphor"), TypeScale.Phone, File(OUT, "new-sheet-phosphor.png")) {
            sheet()(Breakpoint.Portrait)
        }
        render(
            PORTRAIT.first, PORTRAIT.second, themeOf("soft"), TypeScale.Phone,
            File(OUT, "new-sheet-light.png"), Ground.Light,
        ) { sheet()(Breakpoint.Portrait) }
    }

    // A node without `manage`, or a read-only device, never reaches the sheet — but a node that
    // has offered no agent kinds does, and it has to say so rather than showing a stale list.
    @Test
    fun theSheetSaysSoWhenTheNodeOfferedNoKinds() {
        render(PORTRAIT.first, PORTRAIT.second, themeOf("soft"), TypeScale.Phone, File(OUT, "new-sheet-no-kinds.png")) {
            NewSheet(
                breakpoint = Breakpoint.Portrait,
                node = NODE,
                pane = null,
                nodes = listOf(NODE),
                caps = null,
                outcome = null,
                onManage = {},
                onNode = {},
                onNodePicker = {},
                onDismiss = {},
            )
        }
    }

    // A component nothing renders is not done. This draws the herd exactly as the app does and
    // asserts the entry point is there — and that it is absent when the node or the role says no.
    @Test
    fun theHerdHeaderCarriesTheEntryPointOnlyWhenTheNodeAllowsIt() {
        val herd = Herd(
            nodes = listOf(NODE),
            panes = listOf(PANE),
            known = true,
        )
        val opened = mutableListOf<String?>()
        val allowed = object : ManageIo {
            override val enabled = true
            override fun openNew(paneId: String?) { opened += paneId }
            override fun openActions(paneId: String) = Unit
        }
        val refused = object : ManageIo {
            override val enabled = false
            override fun openNew(paneId: String?) = fail("a refused node must not offer it")
            override fun openActions(paneId: String) = fail("a refused node must not offer it")
        }
        val withManage = render(
            PORTRAIT.first, PORTRAIT.second, themeOf("soft"), TypeScale.Phone,
            File(OUT, "herd-with-new.png"),
        ) {
            CompositionLocalProvider(LocalManage provides allowed) {
                HerdPortrait(herd, ConnectionStatus.Live("full"), 0.0, 0.4, emptyList(), {}, null)
            }
        }
        val without = render(
            PORTRAIT.first, PORTRAIT.second, themeOf("soft"), TypeScale.Phone,
            File(OUT, "herd-without-new.png"),
        ) {
            CompositionLocalProvider(LocalManage provides refused) {
                HerdPortrait(herd, ConnectionStatus.Live("full"), 0.0, 0.4, emptyList(), {}, null)
            }
        }
        assertTrue(
            !withManage.encodeToData()!!.bytes.contentEquals(without.encodeToData()!!.bytes),
            "the + must be visible with manage and absent without it",
        )
    }

    @Test
    fun thePaneActionsRender() {
        val sent = mutableListOf<ManageOp>()
        render(PORTRAIT.first, PORTRAIT.second, themeOf("soft"), TypeScale.Phone, File(OUT, "pane-actions.png")) {
            PaneActionsSheet(Breakpoint.Portrait, PANE, null, { sent += it }, {})
        }
    }
}

private val PORTRAIT = 390.dp to 844.dp
private val LANDSCAPE = 844.dp to 390.dp
private val DESKTOP = 1440.dp to 900.dp
