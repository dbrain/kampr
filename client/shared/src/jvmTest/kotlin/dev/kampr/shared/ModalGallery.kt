package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.asSkiaBitmap
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SkikoComposeUiTest
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.v2.runSkikoComposeUiTest
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.ui.HerdPortrait
import dev.kampr.shared.ui.HerdSidebar
import dev.kampr.shared.ui.LocalConnectionStatus
import dev.kampr.shared.ui.LocalManage
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.ManageIo
import dev.kampr.shared.ui.PaneScreenDesktop
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.SafeArea
import dev.kampr.shared.wire.ContainerInfo
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.ServerMsg
import org.jetbrains.skia.EncodedImageFormat
import org.jetbrains.skia.Image
import java.io.File

internal val GALLERY = File("build/artboards/gallery")

internal enum class Frame(val width: Int, val height: Int, val density: Float, val scale: TypeScale, val bars: SafeArea) {
    Portrait(390, 844, 2.625f, TypeScale.Phone, BARS),
    Desktop(1280, 800, 1.5f, TypeScale.Desk, SafeArea(0.dp, 0.dp)),
    ;

    val tag: String get() = name.lowercase()
}

internal const val GALLERY_NOW_SEC = 1_794_220_000L

internal const val G_HUB = "01JHUB"
internal const val G_SUN = "01JSUN"
internal const val G_NAS = "01JNAS"

internal val GALLERY_CONTAINERS = listOf(
    ContainerInfo(
        id = "c0ffee01",
        name = "kobbler-postgres-1",
        image = "postgres:18.6",
        state = "running",
        health = "unhealthy",
        startedAt = GALLERY_NOW_SEC - 600,
        restarts = 2,
        project = "kobbler",
        service = "postgres",
        ports = listOf("5432→5432/tcp"),
    ),
    ContainerInfo(
        id = "c0ffee02",
        name = "kobbler-web-1",
        image = "ghcr.io/dbrain/kobbler:0.4.1",
        state = "exited",
        exitCode = 137,
        oom = true,
        finishedAt = GALLERY_NOW_SEC - 300,
        project = "kobbler",
        service = "web",
    ),
)

internal val GALLERY_NODES = listOf(
    NodeInfo(
        id = G_HUB, name = "comingclean", kind = "local", online = true, rttMs = 0.4, herdrVersion = "0.8.2", build = "0.1.142",
        native = true, muffit = true, docker = true, containers = GALLERY_CONTAINERS,
    ),
    NodeInfo(id = G_SUN, name = "sungrow-pi", kind = "peer", online = true, rttMs = 208.0, herdrVersion = "0.8.2", build = "0.1.142"),
    NodeInfo(
        id = G_NAS,
        name = "nas",
        kind = "peer",
        online = false,
        herdrVersion = "0.8.2",
        build = "0.1.141",
        detail = "nas is not connected: connection reset by peer",
        reachable = false,
    ),
)

internal val G_CLAUDE = PaneInfo(
    id = "$G_HUB/w3:p2",
    nodeId = G_HUB,
    workspaceId = "$G_HUB/w3",
    tabId = "$G_HUB/w3:t1",
    workspace = "kampr",
    tab = "1",
    cwd = "/home/dbrain/dev/kampr",
    title = "Wire the hooks socket",
    agent = "claude",
    agentStatus = "blocked",
    cols = 94,
    rows = 40,
    hasConversation = true,
    converses = true,
)

internal val G_SHELL = PaneInfo(
    id = "$G_HUB/w3:p3", nodeId = G_HUB, workspaceId = "$G_HUB/w3", tabId = "$G_HUB/w3:t1", workspace = "kampr", tab = "1",
    cwd = "/home/dbrain/dev/kampr", agentStatus = "idle", cols = 94, rows = 40, cmd = "cargo", argv = "cargo test -p kampr-node",
)

internal val G_CODEX = PaneInfo(
    id = "$G_HUB/w4:p1", nodeId = G_HUB, workspaceId = "$G_HUB/w4", tabId = "$G_HUB/w4:t1", workspace = "kob", tab = "1",
    cwd = "/home/dbrain/dev/tinyfiddler/kob", agent = "codex", agentStatus = "working", cols = 80, rows = 30,
)

internal val G_NATIVE = PaneInfo(
    id = "$G_HUB/n:s1", nodeId = G_HUB, cwd = "/srv/kobbler", label = "deploy-watch", agentStatus = "idle", cols = 80, rows = 24,
    provider = "native", sessionId = "s1", cmd = "journalctl", argv = "journalctl -fu kobbler",
)

internal val G_DONE = PaneInfo(
    id = "$G_SUN/w1:p1", nodeId = G_SUN, workspaceId = "$G_SUN/w1", tabId = "$G_SUN/w1:t1", workspace = "sungrow", tab = "1",
    cwd = "/home/pi/dev/sungrow", agent = "claude", agentStatus = "done", cols = 80, rows = 24, hasConversation = true,
)

internal val GALLERY_HERD = Herd(
    nodes = GALLERY_NODES,
    panes = listOf(G_CLAUDE, G_SHELL, G_CODEX, G_NATIVE, G_DONE),
    known = true,
)

internal object GalleryManage : ManageIo {
    override val enabled = true
    override fun openNew(paneId: String?) = Unit
    override fun openActions(paneId: String) = Unit
    override val browses: Boolean get() = true
}

internal fun paintedPane(id: String): PaneState = PaneState(id, StyleTable()).also {
    it.applyReset(
        ServerMsg.GridReset(
            pane = id,
            cols = 80,
            rows = 24,
            rowsData = listOf(RowDiff(23, listOf(Run(0, "$ ")))),
            cursor = Cursor(2, 23, true),
            links = emptyList(),
        ),
    )
}

@Composable
internal fun PaneBehind(frame: Frame, info: PaneInfo = G_CLAUDE, pane: PaneState = PaneState(info.id, StyleTable()), readOnly: Boolean = false) {
    when (frame) {
        Frame.Portrait -> PaneScreenMobile(
            pane = pane,
            info = info,
            view = PaneView.Terminal,
            surfaces = BlankSurfaces,
            landscape = false,
            readOnly = readOnly,
            onBack = {},
            onView = {},
            modifier = Modifier.fillMaxSize(),
        )

        Frame.Desktop -> Row(Modifier.fillMaxSize()) {
            HerdBehindSidebar(Modifier.width(300.dp).fillMaxHeight())
            Box(Modifier.weight(1f).fillMaxHeight()) {
                PaneScreenDesktop(
                    pane = pane,
                    info = info,
                    view = PaneView.Terminal,
                    surfaces = BlankSurfaces,
                    readOnly = readOnly,
                    onView = {},
                    modifier = Modifier.fillMaxSize(),
                )
            }
        }
    }
}

@Composable
private fun HerdBehindSidebar(modifier: Modifier) {
    Box(modifier) {
        HerdSidebar(
            GALLERY_HERD, ConnectionStatus.Live("full"), GALLERY_NOW_SEC * 1000.0, 0.4, emptyList(), G_CLAUDE.id,
            "pixel-6", "full access", {}, {},
        )
    }
}

@Composable
internal fun HerdBehind(frame: Frame) {
    when (frame) {
        Frame.Portrait -> HerdPortrait(GALLERY_HERD, ConnectionStatus.Live("full"), GALLERY_NOW_SEC * 1000.0, 0.4, emptyList(), {}, null)
        Frame.Desktop -> PaneBehind(frame)
    }
}

// The scene renders through the ui-test harness rather than `render`, because several of these
// surfaces only exist after a press (a menu's confirm step, a popup) and `captureToImage` on the
// scene includes popup layers.
@OptIn(ExperimentalTestApi::class)
internal fun shoot(
    name: String,
    frame: Frame,
    ground: Ground = Ground.Dark,
    act: SkikoComposeUiTest.() -> Unit = {},
    content: @Composable (Frame) -> Unit,
): File {
    val file = File(GALLERY, "$name-${frame.tag}${if (ground == Ground.Light) "-light" else ""}.png")
    val density = Density(frame.density)
    runSkikoComposeUiTest(Size(frame.width * frame.density, frame.height * frame.density), density) {
        mainClock.autoAdvance = false
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(themeOf("soft"), frame.scale, ground),
                LocalSafeArea provides frame.bars,
                LocalManage provides GalleryManage,
                LocalConnectionStatus provides ConnectionStatus.Live("full"),
            ) {
                Box(Modifier.fillMaxSize()) { content(frame) }
            }
        }
        mainClock.advanceTimeBy(500)
        act()
        mainClock.advanceTimeBy(500)
        val bitmap = captureToImage().asSkiaBitmap()
        file.parentFile.mkdirs()
        file.writeBytes(requireNotNull(Image.makeFromBitmap(bitmap).encodeToData(EncodedImageFormat.PNG)).bytes)
    }
    return file
}

@OptIn(ExperimentalTestApi::class)
internal fun SkikoComposeUiTest.tap(said: String, substring: Boolean = true) {
    val nodes = onAllNodes(hasContentDescription(said, substring = substring).or(hasText(said, substring = substring)))
    nodes[0].performSemanticsAction(SemanticsActions.OnClick)
    mainClock.advanceTimeBy(400)
}
