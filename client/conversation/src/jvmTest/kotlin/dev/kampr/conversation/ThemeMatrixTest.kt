package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.ImageComposeScene
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.TriageItem
import dev.kampr.shared.net.DeviceRecord
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.net.SetupStatus
import dev.kampr.shared.theme.AllThemes
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.theme.LocalGround
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.LocalConnectionStatus
import dev.kampr.shared.theme.ThemeMode
import dev.kampr.shared.theme.ThemeSpec
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.AppearanceScreen
import dev.kampr.shared.ui.DevicesScreen
import dev.kampr.shared.ui.HerdPortrait
import dev.kampr.shared.ui.HerdSidebar
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneScreenDesktop
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.SetupScreen
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.Security
import java.io.File
import kotlin.test.Test
import kotlin.test.assertTrue

private val OUT = File("build/artboards/matrix")

private const val NOW = 1_787_000_000_000.0

private val herd = Herd(
    nodes = listOf(
        NodeInfo("01JNODE", "studio", "local", true, 4.0, "0.14.2"),
        NodeInfo("01JPEER", "pi-shed", "peer", true, 41.0, "0.14.1"),
    ),
    panes = listOf(
        PaneInfo(
            id = PANE_ID, nodeId = "01JNODE", workspace = "kampr", tab = "1",
            cwd = "/home/dbrain/dev/kampr", agent = "claude", agentStatus = "blocked",
            cols = 74, rows = 30, scrollbackRows = 0, hasConversation = true,
        ),
        PaneInfo(
            id = "01JNODE/w3:p3", nodeId = "01JNODE", workspace = "kampr", tab = "2",
            cwd = "/home/dbrain/dev/kampr/crates", agent = "codex", agentStatus = "working",
            cols = 120, rows = 40, hasConversation = true,
        ),
        PaneInfo(
            id = "01JNODE/w4:p1", nodeId = "01JNODE", workspace = "notes", tab = "1",
            cwd = "/home/dbrain/notes", agent = null, agentStatus = "unknown",
            cols = 80, rows = 24, scrollbackRows = 812,
        ),
        PaneInfo(
            id = "01JPEER/w1:p1", nodeId = "01JPEER", workspace = "shed", tab = "1",
            cwd = "/srv/shed", agent = "claude", agentStatus = "done",
            cols = 100, rows = 30, hasConversation = true,
        ),
    ),
    known = true,
)

private val devices = listOf(
    DeviceRecord("d1", "Pixel 9", "full", createdAt = 1_786_000_000, lastSeenAt = 1_787_269_000),
    DeviceRecord("d2", "MacBook", "full", createdAt = 1_785_000_000, lastSeenAt = 1_787_100_000),
    DeviceRecord("d3", "shared iPad", "readonly", createdAt = 1_784_000_000, lastSeenAt = 1_786_500_000),
)

private val triage = listOf(TriageItem(herd.panes.first(), "Do you want to make this edit?"))

private val setup = SetupStatus("http://192.168.1.24:8790", 3, "0.5.0", passkeys = true, installable = true)

// Renders the same screen once per theme, side by side, under one ground. Four narrow columns
// is what makes a token that quietly stopped resolving visible: it shows up as the one column
// that did not move.
private fun sheet(
    width: Dp,
    height: Dp,
    ground: Ground,
    scale: TypeScale,
    name: String,
    content: @Composable (ThemeSpec) -> Unit,
) {
    val density = Density(2f)
    val scene = ImageComposeScene(
        width = with(density) { (width * AllThemes.size).roundToPx() },
        height = with(density) { height.roundToPx() },
        density = density,
    ) {
        CompositionLocalProvider(LocalPaneIo provides RecordingIo, LocalGround provides ground) {
            Row(Modifier.fillMaxSize()) {
                for (spec in AllThemes) {
                    CompositionLocalProvider(
                        LocalTokens provides tokensFor(spec, scale, ground),
                        LocalConnectionStatus provides ConnectionStatus.Live("full"),
                    ) {
                        Box(Modifier.width(width).height(height).background(Kampr.tokens.color.bg)) {
                            content(spec)
                        }
                    }
                }
            }
        }
    }
    try {
        scene.render()
        val image = scene.render()
        OUT.mkdirs()
        File(OUT, "$name-${ground.name.lowercase()}.png")
            .writeBytes(requireNotNull(image.encodeToData()).bytes)
        assertTrue(image.width > 0 && image.height > 0, name)
    } finally {
        scene.close()
    }
}

private val phone = 390.dp to 844.dp
private val desk = 1440.dp to 900.dp

class ThemeMatrixTest {
    private fun eachGround(body: (Ground) -> Unit) = Ground.entries.forEach(body)

    @Test
    fun herdSheet() = eachGround { ground ->
        sheet(phone.first, phone.second, ground, TypeScale.Phone, "herd") {
            HerdPortrait(herd, ConnectionStatus.Live("full"), NOW, 4.0, triage, {}, {})
        }
    }

    @Test
    fun conversationSheet() = eachGround { ground ->
        sheet(phone.first, phone.second, ground, TypeScale.Phone, "conversation") {
            val (_, pane) = demoPane(RICH_CONVO)
            PaneScreenMobile(
                pane = pane,
                info = demoInfo(),
                view = PaneView.Conversation,
                surfaces = ConversationSurfaces(),
                landscape = false,
                readOnly = false,
                onBack = {},
                onView = {},
                onAnswer = {},
            )
        }
    }

    @Test
    fun setupSheet() = eachGround { ground ->
        sheet(phone.first, phone.second, ground, TypeScale.Phone, "setup") {
            SetupScreen(
                status = setup,
                security = Security(tier = 1, encrypted = false, passkeys = true, installable = true),
                running = true,
                endpoint = Endpoint("http://192.168.1.24:8790", "token"),
                nodes = herd.nodes,
                pairingCode = "4821-9930",
                pairingError = null,
                onConnect = {},
                onPairingCode = {},
                onDevices = {},
                onAppearance = {},
                onNotifications = {},
                onPasskeys = {},
            )
        }
    }

    @Test
    fun devicesSheet() = eachGround { ground ->
        sheet(phone.first, phone.second, ground, TypeScale.Phone, "devices") {
            DevicesScreen(devices, "d1", NOW, {}, {}, {})
        }
    }

    @Test
    fun appearanceSheet() = eachGround { ground ->
        sheet(phone.first, phone.second, ground, TypeScale.Phone, "appearance") { spec ->
            AppearanceScreen(
                selected = spec.id,
                mode = if (ground == Ground.Light) ThemeMode.Light else ThemeMode.Dark,
                onSelect = {},
                onMode = {},
                onBack = {},
            )
        }
    }

    @Test
    fun desktopSheet() = eachGround { ground ->
        sheet(desk.first, desk.second, ground, TypeScale.Desk, "desktop") {
            val (_, pane) = demoPane(RICH_CONVO)
            Row(Modifier.fillMaxSize()) {
                HerdSidebar(herd, ConnectionStatus.Live("full"), NOW, 4.0, triage, PANE_ID, "studio", "local · 4 ms", {}, {})
                Column(Modifier.fillMaxSize()) {
                    PaneScreenDesktop(
                        pane = pane,
                        info = demoInfo(),
                        view = PaneView.Conversation,
                        surfaces = ConversationSurfaces(),
                        readOnly = false,
                        onView = {},
                        onAnswer = {},
                    )
                }
            }
        }
    }
}
