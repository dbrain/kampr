package dev.kampr.shared

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.PaneGone
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.model.paneTitle
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.CommandPalette
import dev.kampr.shared.ui.ConnectPanel
import dev.kampr.shared.ui.DockerSheet
import dev.kampr.shared.ui.FindSheet
import dev.kampr.shared.ui.FleetScreen
import dev.kampr.shared.ui.HerdPortrait
import dev.kampr.shared.ui.MenuAnchor
import dev.kampr.shared.ui.NewSheet
import dev.kampr.shared.ui.NodeListSheet
import dev.kampr.shared.ui.PaneActionsSheet
import dev.kampr.shared.ui.PaneMenu
import dev.kampr.shared.ui.PaneScreenDesktop
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.RunSheet
import dev.kampr.shared.ui.paletteItems
import dev.kampr.shared.wire.ContainerMore
import dev.kampr.shared.wire.DockerDetail
import dev.kampr.shared.wire.DockerEvent
import dev.kampr.shared.wire.DockerMount
import dev.kampr.shared.wire.DockerStats
import dev.kampr.shared.wire.FindMatch
import dev.kampr.shared.wire.FleetCommand
import dev.kampr.shared.wire.FleetInfo
import dev.kampr.shared.wire.HealthProbe
import dev.kampr.shared.wire.LogLine
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.Question
import dev.kampr.shared.wire.QuestionOption
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.SessionInfo
import kotlin.test.Test

private val BOTH = listOf(Frame.Portrait, Frame.Desktop)

private fun Frame.breakpoint() = if (this == Frame.Portrait) Breakpoint.Portrait else Breakpoint.Desktop

private val CAPS = ServerMsg.NodeCaps(
    node = G_HUB,
    agentKinds = listOf("aider", "amp", "claude", "codex", "gemini", "goose", "opencode", "qwen"),
    sessions = listOf(SessionInfo("default", true), SessionInfo("agents", false)),
)

private val DETAIL = DockerDetail(
    GALLERY_CONTAINERS[0],
    ContainerMore(
        env = listOf("POSTGRES_DB=kobbler", "POSTGRES_USER=kobbler"),
        mounts = listOf(DockerMount("/srv/pg", "/var/lib/postgresql", "rw")),
        networks = listOf("kobbler_default"),
        healthLog = listOf(HealthProbe(exitCode = 1, output = "pg_isready: no response")),
        logs = listOf(LogLine("stderr", "FATAL: the database system is starting up")),
        stats = DockerStats(cpuPct = 12.5, memUsed = 44_040_192, memLimit = 1_073_741_824, pids = 9),
        events = listOf(DockerEvent(GALLERY_NOW_SEC - 300, "oom"), DockerEvent(GALLERY_NOW_SEC - 300, "die", exitCode = 137)),
    ),
)

private val BOOK = ServerMsg.FleetBook(
    recent = listOf(FleetCommand("r1", listOf("sudo", "pacman", "-Syu", "--noconfirm"), at = GALLERY_NOW_SEC - 3600)),
    saved = listOf(
        FleetCommand("s1", listOf("df", "-h", "/"), label = "Disk space"),
        FleetCommand("s2", listOf("systemctl", "--failed")),
    ),
)

private val UPGRADE = Question(
    prompt = ":: Proceed with installation? [Y/n]",
    shape = "confirm",
    options = listOf(QuestionOption("y", "Y"), QuestionOption("n", "n")),
    defaultKey = "y",
)

private fun fleetPane(n: Int, node: String, question: Question) = PaneInfo(
    id = "$node/fleet-$n",
    nodeId = node,
    fleet = FleetInfo(cohort = "c1", command = "sudo pacman -Syu", state = "waiting", question = question, startedUnix = GALLERY_NOW_SEC - 90),
)

private val FLEET_HERD = Herd(
    nodes = GALLERY_NODES + NodeInfo(id = "01JBOX", name = "buildbox", kind = "peer", online = true, rttMs = 12.0),
    panes = listOf(
        fleetPane(1, G_HUB, UPGRADE),
        fleetPane(2, G_SUN, UPGRADE),
        fleetPane(3, "01JBOX", UPGRADE),
        fleetPane(4, G_NAS, UPGRADE.copy(prompt = ":: Replace iptables with iptables-nft? [y/N]", defaultKey = "n")),
    ),
    known = true,
)

private val FOUND = ServerMsg.Found(
    pane = G_CLAUDE.id,
    query = "panicked",
    matches = listOf(
        FindMatch(412, text = "thread 'live::hooks_socket' panicked at crates/kampr-node/tests/it/live.rs:881:9"),
        FindMatch(97, text = "thread 'main' panicked at 'called `Result::unwrap()` on an `Err` value'"),
        FindMatch(3, text = "note: run with `RUST_BACKTRACE=1` to see a backtrace"),
    ),
    total = 14,
    current = 0,
)

@OptIn(ExperimentalTestApi::class)
class ModalGalleryArtboardTest {
    @Test
    fun paneMenu() {
        val title = "Wire the hooks socket"
        for (frame in BOTH) {
            val anchor = if (frame == Frame.Desktop) MenuAnchor(120.dp, 150.dp) else null
            val menu: @androidx.compose.runtime.Composable (Frame) -> Unit = {
                PaneBehind(it)
                PaneMenu(it.breakpoint(), G_CLAUDE, anchor, null, {}, {}, {})
            }
            shoot("pane-menu", frame, content = menu)
            shoot("pane-menu-rename", frame, act = { tap("Rename $title") }, content = menu)
            shoot("pane-menu-close", frame, act = { tap("Close $title") }, content = menu)
            shoot("pane-menu-native", frame) {
                PaneBehind(it, G_NATIVE)
                PaneMenu(it.breakpoint(), G_NATIVE, anchor, null, {}, {}, {}, desk = true)
            }
        }
        shoot("pane-menu", Frame.Desktop, Ground.Light) {
            PaneBehind(it)
            PaneMenu(it.breakpoint(), G_CLAUDE, MenuAnchor(120.dp, 150.dp), null, {}, {}, {})
        }
    }

    @Test
    fun paneActions() {
        for (frame in BOTH) {
            shoot("pane-actions", frame) {
                PaneBehind(it)
                PaneActionsSheet(it.breakpoint(), G_CLAUDE, null, {}, {}, GALLERY_HERD.panes, onFind = {}, onFiles = {})
            }
            shoot("pane-actions-close-tab", frame, act = { tap("Close this tab") }) {
                PaneBehind(it)
                PaneActionsSheet(it.breakpoint(), G_CLAUDE, null, {}, {}, GALLERY_HERD.panes, onFind = {}, onFiles = {})
            }
            shoot("pane-actions-native", frame) {
                PaneBehind(it, G_NATIVE)
                PaneActionsSheet(it.breakpoint(), G_NATIVE, null, {}, {}, GALLERY_HERD.panes, onFind = {}, onFiles = {}, desk = true)
            }
        }
        shoot("pane-actions", Frame.Portrait, Ground.Light) {
            PaneBehind(it)
            PaneActionsSheet(it.breakpoint(), G_CLAUDE, null, {}, {}, GALLERY_HERD.panes, onFind = {}, onFiles = {})
        }
    }

    @Test
    fun newSheet() {
        val sheet: @androidx.compose.runtime.Composable (Frame) -> Unit = {
            PaneBehind(it)
            NewSheet(
                breakpoint = it.breakpoint(),
                node = GALLERY_NODES[0],
                pane = G_CLAUDE,
                nodes = GALLERY_NODES,
                caps = CAPS,
                outcome = null,
                onManage = {},
                onNode = {},
                onNodePicker = {},
                onDismiss = {},
                panes = GALLERY_HERD.panes,
                muffit = true,
            )
        }
        for (frame in BOTH) {
            shoot("new-sheet", frame, content = sheet)
            shoot("new-sheet-kampr-session", frame, act = { tap("Kampr session, A shell") }, content = sheet)
            shoot("new-sheet-machine", frame, act = { tap("Change machine") }, content = sheet)
            shoot("new-sheet-herdr", frame, act = { tap("Herdr") }, content = sheet)
            shoot("new-sheet-herdr-workspace", frame, act = {
                tap("Herdr")
                tap("Workspace, A herdr workspace")
            }, content = sheet)
        }
        shoot("new-sheet", Frame.Portrait, Ground.Light, content = sheet)
    }

    @Test
    fun dockerSheet() {
        for (frame in BOTH) {
            for ((tab, act) in listOf<Pair<String, androidx.compose.ui.test.SkikoComposeUiTest.() -> Unit>>(
                "info" to {},
                "logs" to { tap("Logs tab") },
                "stats" to { tap("Stats tab") },
            )) {
                shoot("docker-sheet-$tab", frame, act = act) {
                    HerdBehind(it)
                    DockerSheet(it.breakpoint(), GALLERY_NODES[0], GALLERY_CONTAINERS[0], DETAIL, null, GALLERY_NOW_SEC * 1000.0, {}, {})
                }
            }
            shoot("docker-sheet-remove", frame, act = { tap("Remove this container") }) {
                HerdBehind(it)
                DockerSheet(it.breakpoint(), GALLERY_NODES[0], GALLERY_CONTAINERS[1], DETAIL.copy(entry = GALLERY_CONTAINERS[1]), null, GALLERY_NOW_SEC * 1000.0, {}, {})
            }
            shoot("docker-sheet-exited", frame) {
                HerdBehind(it)
                DockerSheet(
                    it.breakpoint(),
                    GALLERY_NODES[0],
                    GALLERY_CONTAINERS[1],
                    DETAIL.copy(entry = GALLERY_CONTAINERS[1]),
                    "docker: Error response from daemon: conflict",
                    GALLERY_NOW_SEC * 1000.0,
                    {},
                    {},
                )
            }
        }
    }

    @Test
    fun fleet() {
        for (frame in BOTH) {
            shoot("fleet-run-sheet", frame) {
                FleetScreen(FLEET_HERD, it.breakpoint(), {}, { _, _ -> }, {}, {}, canRun = true, book = BOOK)
                RunSheet(hosts = 3, book = BOOK, breakpoint = it.breakpoint(), onCancel = {}, onRun = {}, onBook = {})
            }
            shoot("fleet-run-sheet-staged", frame) {
                FleetScreen(FLEET_HERD, it.breakpoint(), {}, { _, _ -> }, {}, {}, canRun = true, book = BOOK)
                RunSheet(hosts = 3, book = BOOK, staged = BOOK.saved[0], breakpoint = it.breakpoint(), onCancel = {}, onRun = {}, onBook = {})
            }
            shoot("fleet-broadcast-confirm", frame, act = { tap("Y", substring = false) }) {
                FleetScreen(FLEET_HERD, it.breakpoint(), {}, { _, _ -> }, {}, {}, canRun = true, book = BOOK, modifier = Modifier.fillMaxSize())
            }
        }
    }

    @Test
    fun nodeList() {
        for (frame in BOTH) {
            shoot("node-list-sheet", frame) {
                HerdBehind(it)
                NodeListSheet(GALLERY_NODES, it.breakpoint(), {}, {})
            }
        }
    }

    @Test
    fun findSheet() {
        for (frame in BOTH) {
            shoot("find-sheet", frame) {
                PaneBehind(it)
                FindSheet(G_CLAUDE.id, paneTitle(G_CLAUDE), it.breakpoint(), FOUND, {}, {})
            }
            shoot("find-sheet-empty", frame) {
                PaneBehind(it)
                FindSheet(G_CLAUDE.id, paneTitle(G_CLAUDE), it.breakpoint(), null, {}, {})
            }
        }
    }

    @Test
    fun palette() {
        for (frame in BOTH) {
            shoot("command-palette", frame) {
                HerdBehind(it)
                CommandPalette(paletteItems(GALLERY_HERD, mosaic = true, canCreate = true, canBrowse = true), {}, {})
            }
        }
    }

    @Test
    fun connectPanel() {
        for (frame in BOTH) {
            shoot("connect-panel", frame) {
                Box(Modifier.fillMaxSize().background(Kampr.tokens.color.bg).padding(top = 60.dp, start = 16.dp, end = 16.dp), contentAlignment = Alignment.TopCenter) {
                    ConnectPanel(
                        Endpoint("https://kampr.comingclean.lan:8790"),
                        "Could not reach kampr.comingclean.lan: connection refused",
                        {},
                        onPasskey = {},
                        recent = listOf("https://kampr.comingclean.lan:8790", "http://192.168.1.24:8790"),
                        onScan = {},
                    )
                }
            }
        }
    }

    @Test
    fun nodeOverflowPopup() {
        for (frame in BOTH) {
            shoot("node-overflow-popup", frame, act = { tap("More for comingclean") }) {
                if (it == Frame.Portrait) {
                    HerdPortrait(GALLERY_HERD, ConnectionStatus.Live("full"), GALLERY_NOW_SEC * 1000.0, 0.4, emptyList(), {}, null)
                } else {
                    PaneBehind(it)
                }
            }
        }
    }

    @Test
    fun paneNotices() {
        for (frame in BOTH) {
            shoot("notice-pane-gone", frame) { header(it, G_CLAUDE, paintedPane(G_CLAUDE.id), gone = PaneGone.Shell) }
            shoot("notice-node-gone", frame) { header(it, G_DONE, paintedPane(G_DONE.id), gone = PaneGone.Node) }
            shoot("notice-no-stream", frame) {
                header(it, G_CLAUDE.copy(detail = "herdr's pane.read failed: Broken pipe (os error 32)"), PaneState(G_CLAUDE.id, StyleTable()))
            }
            shoot("notice-watchers", frame, act = { mainClock.advanceTimeBy(2_600) }) {
                header(it, G_CLAUDE.copy(watchers = 3), paintedPane(G_CLAUDE.id))
            }
        }
    }

    @Test
    fun paneHeaders() {
        val states = listOf<Triple<String, PaneInfo, () -> PaneState>>(
            Triple("none", G_CLAUDE.copy(agentStatus = "idle"), { paintedPane(G_CLAUDE.id) }),
            Triple("working", G_CLAUDE.copy(agentStatus = "working"), { paintedPane(G_CLAUDE.id) }),
            Triple("done", G_CLAUDE.copy(agentStatus = "done"), { paintedPane(G_CLAUDE.id) }),
            Triple("blocked", G_CLAUDE, { paintedPane(G_CLAUDE.id) }),
            Triple("quiet", G_CLAUDE.copy(agentStatus = "idle"), { paintedPane(G_CLAUDE.id).also { p -> p.noteStreamStopped() } }),
            Triple("stale", G_CLAUDE.copy(agentStatus = "idle"), { paintedPane(G_CLAUDE.id).also { p -> p.stale = true } }),
            Triple("quiet-working", G_CLAUDE.copy(agentStatus = "working"), { paintedPane(G_CLAUDE.id).also { p -> p.noteStreamStopped() } }),
        )
        for (frame in BOTH) {
            for ((name, info, pane) in states) {
                shoot("pane-header-$name", frame) { header(it, info, pane()) }
            }
            shoot("pane-header-readonly-stale-unsent", frame) {
                header(
                    it,
                    G_CLAUDE.copy(agentStatus = "idle"),
                    paintedPane(G_CLAUDE.id).also { p ->
                        p.stale = true
                        repeat(3) { p.noteUndelivered() }
                    },
                    readOnly = true,
                )
            }
        }
    }

    @androidx.compose.runtime.Composable
    private fun header(frame: Frame, info: PaneInfo, pane: PaneState, readOnly: Boolean = false, gone: PaneGone? = null) {
        when (frame) {
            Frame.Portrait -> PaneScreenMobile(
                pane = pane, info = info, view = PaneView.Terminal, surfaces = BlankSurfaces, landscape = false,
                readOnly = readOnly, gone = gone, onBack = {}, onView = {}, modifier = Modifier.fillMaxSize(),
            )

            Frame.Desktop -> PaneScreenDesktop(
                pane = pane,
                info = info,
                view = PaneView.Terminal,
                surfaces = BlankSurfaces,
                readOnly = readOnly,
                gone = gone,
                onView = {},
                modifier = Modifier.fillMaxSize(),
            )
        }
    }
}
