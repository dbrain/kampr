package dev.kampr.mosaic

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.ImageComposeScene
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.platform.Font
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.net.KamprConnection
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.theme.FamilyId
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalGround
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.ThemeSpec
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.on
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Style
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import java.io.File

val OUT: File = File("build/artboards")

object ArtboardIo : PaneIo {
    val sent = mutableListOf<ClientMsg>()
    override fun send(msg: ClientMsg) {
        sent += msg
    }
    override fun prefs(paneId: String): PanePrefs = PanePrefs()
}

// Compose resources resolve fonts asynchronously and a headless render never waits for that, so
// the artboards load the same .ttf files straight off disk.
private fun family(id: FamilyId): FontFamily {
    val dir = File("../shared/src/commonMain/composeResources/font")
    fun face(name: String, weight: FontWeight) = Font(name, File(dir, "$name.ttf").readBytes(), weight)
    return when (id) {
        FamilyId.Manrope -> FontFamily(
            face("manrope_400", FontWeight.W400), face("manrope_500", FontWeight.W500),
            face("manrope_600", FontWeight.W600), face("manrope_700", FontWeight.W700),
            face("manrope_800", FontWeight.W800),
        )
        FamilyId.IbmPlexMono -> FontFamily(
            face("ibmplexmono_400", FontWeight.W400), face("ibmplexmono_500", FontWeight.W500),
            face("ibmplexmono_600", FontWeight.W600),
        )
        FamilyId.JetBrainsMono -> FontFamily(
            face("jetbrainsmono_400", FontWeight.W400), face("jetbrainsmono_500", FontWeight.W500),
            face("jetbrainsmono_700", FontWeight.W700),
        )
        FamilyId.InstrumentSans -> FontFamily(
            face("instrumentsans_400", FontWeight.W400), face("instrumentsans_500", FontWeight.W500),
            face("instrumentsans_600", FontWeight.W600),
        )
        FamilyId.Archivo -> FontFamily(
            face("archivo_500", FontWeight.W500), face("archivo_700", FontWeight.W700),
            face("archivo_900", FontWeight.W900),
        )
    }
}

fun tokensFor(spec: ThemeSpec, scale: TypeScale, ground: Ground): KamprTokens {
    val grounded = spec.on(ground)
    val fonts = KamprFonts(family(grounded.ui), family(grounded.mono), family(FamilyId.JetBrainsMono))
    return KamprTokens(grounded, fonts, typography(fonts, grounded.label, scale))
}

fun renderArtboard(
    width: Dp,
    height: Dp,
    spec: ThemeSpec,
    scale: TypeScale,
    file: File,
    ground: Ground = Ground.Dark,
    content: @Composable () -> Unit,
) {
    val density = Density(2f)
    val scene = ImageComposeScene(
        width = with(density) { width.roundToPx() },
        height = with(density) { height.roundToPx() },
        density = density,
    ) {
        CompositionLocalProvider(
            LocalTokens provides tokensFor(spec, scale, ground),
            LocalGround provides ground,
            LocalPaneIo provides ArtboardIo,
        ) {
            Box(Modifier.fillMaxSize()) { content() }
        }
    }
    try {
        // Twice: the first pass measures, the second paints what the measurement settled on.
        scene.render()
        val image = scene.render()
        file.parentFile.mkdirs()
        file.writeBytes(requireNotNull(image.encodeToData()).bytes)
    } finally {
        scene.close()
    }
}

class Fixture {
    val store = KamprStore()
    val connection = KamprConnection(CoroutineScope(Job()), store)
    val mosaic = MosaicState(MemoryPrefs(), connection)
}

const val HUB = "01JHUB"
const val AGENTS = "01JHUB.agents"
const val SUN = "01JSUN"
const val NAS = "01JNAS"

val CLAUDE = "$HUB/w3:p2"
val CODEX = "$AGENTS/w1:p1"
val SUNGROW = "$SUN/w1:p1"
val DOGE = "$NAS/w1:p1"

// 208 ms is what a real peer on a slow link measures, and it has to read differently from the
// 0.4 ms pane on the same screen or P4.9 has not been done.
fun herdMessage(sunOnline: Boolean = true): ServerMsg.Herd = ServerMsg.Herd(
    nodes = listOf(
        NodeInfo(HUB, "comingclean", "local", true, 0.4, "0.8.2", "0.1.0"),
        NodeInfo(AGENTS, "comingclean/agents", "local", true, 0.6, "0.8.2", "0.1.0"),
        NodeInfo(
            SUN, "sungrow-pi", "peer", sunOnline,
            if (sunOnline) 208.0 else null, "0.8.2", "0.1.0",
            detail = if (sunOnline) null else "sungrow-pi is not connected: connection reset by peer",
        ),
        NodeInfo(NAS, "nas", "peer", true, 6.0, "0.8.2", "0.1.0"),
    ),
    panes = listOf(
        PaneInfo(CLAUDE, HUB, workspace = "kampr", cwd = "~/dev/kampr", agent = "claude", agentStatus = "blocked", cols = 74, rows = 20),
        PaneInfo(CODEX, AGENTS, workspace = "kob", cwd = "~/dev/tinyfiddler/kob", agent = "codex", agentStatus = "working", cols = 80, rows = 20),
        PaneInfo(SUNGROW, SUN, workspace = "sungrow", cwd = "~/dev/sungrow", agent = "claude", agentStatus = "done", cols = 80, rows = 20),
        PaneInfo(DOGE, NAS, workspace = "houseofdoge", cwd = "~/srv/doge", agent = null, agentStatus = "idle", cols = 80, rows = 20),
    ),
)

private val STYLES = listOf(
    Style(),
    Style(fg = rgb(0xABDFA7)),
    Style(fg = rgb(0x7F7F7F), dim = true),
    Style(fg = rgb(0xF6E2B7)),
    Style(fg = rgb(0x87AFD7)),
)

private fun rgb(hex: Int) = dev.kampr.shared.wire.ColorSpec.Rgb((hex shr 16) and 0xFF, (hex shr 8) and 0xFF, hex and 0xFF)

// A real pane's cursor sits at the bottom of its grid with the blank rows above it, not below —
// content pinned to the top is a fixture artefact that would hide the letterboxing rule.
fun Fixture.paint(paneId: String, cols: Int, rows: Int, lines: List<Pair<Int, String>>) {
    val top = (rows - lines.size).coerceAtLeast(0)
    store.accept(ServerMsg.Styles(0, STYLES))
    store.accept(
        ServerMsg.GridReset(
            pane = paneId,
            cols = cols,
            rows = rows,
            rowsData = lines.mapIndexedNotNull { index, (style, text) ->
                text.takeIf { it.isNotEmpty() }?.let { RowDiff(top + index, listOf(Run(style, it))) }
            },
            cursor = Cursor(lines.lastOrNull()?.second?.length ?: 0, rows - 1, true),
            links = emptyList(),
        ),
    )
    // A live pane has a ring behind it; zoom is computed against the whole surface, so a fixture
    // with no history renders at a size no real pane ever does.
    store.accept(
        ServerMsg.Scrollback(
            pane = paneId,
            fromTop = 0,
            rows = (0 until RING).map { RowDiff(it, listOf(Run(2, HISTORY[it % HISTORY.size]))) },
            totalRows = RING,
            complete = false,
            capped = true,
        ),
    )
}

private const val RING = 40

private val HISTORY = listOf(
    "   Compiling kampr-core v0.1.0 (crates/kampr-core)",
    "   Compiling kampr-node v0.1.0 (crates/kampr-node)",
    "    Finished `dev` profile [unoptimized] in 11.2s",
    "     Running `target/debug/kampr serve`",
    "  ",
)

val CLAUDE_LINES = listOf(
    1 to "● Read bridge/server.ts (412 lines)",
    0 to "",
    2 to "╭─ Edit ────────────────────────────╮",
    1 to "│ 142 +  if (!device.canWrite) {    │",
    1 to "│ 143 +    return json(403, \"ro\")   │",
    2 to "╰───────────────────────────────────╯",
    0 to "",
    0 to "Do you want to make this edit?",
    3 to "❯ 1. Yes   2. Always   3. No",
)

val CODEX_LINES = listOf(
    1 to "• Ran tests for tinyfiddler/kob",
    0 to "",
    1 to "  PASS  src/tuner.test.ts      (28 tests)",
    1 to "  PASS  src/pitch.test.ts      (14 tests)",
    3 to "  RUNS  src/waveform.test.ts",
    0 to "",
    0 to "  Test files  2 passed, 1 running",
    0 to "       Tests  42 passed",
    0 to "",
    2 to "esc to interrupt · 1m 12s",
)

val SUNGROW_LINES = listOf(
    1 to "● Inverter poller is back up.",
    0 to "",
    0 to "  Fixed the modbus timeout — the unit",
    0 to "  drops the connection after 60s idle,",
    0 to "  so the client now reconnects rather",
    0 to "  than waiting on a dead socket.",
    0 to "",
    2 to "  3 files changed, 41 insertions(+)",
    0 to "",
    2 to "〉",
)

val DOGE_LINES = listOf(
    2 to "[13:02] docker compose ps",
    0 to "",
    0 to "NAME             STATUS",
    0 to "doge-web         Up 14 days",
    0 to "doge-db          Up 14 days",
    0 to "doge-cache       Up 14 days",
    0 to "",
    2 to "[13:44]",
)

fun Fixture.fourPanes() {
    store.accept(herdMessage())
    paint(CLAUDE, 74, 20, CLAUDE_LINES)
    paint(CODEX, 80, 20, CODEX_LINES)
    paint(SUNGROW, 80, 20, SUNGROW_LINES)
    paint(DOGE, 80, 20, DOGE_LINES)
    mosaic.attach()
    for (id in listOf(CLAUDE, CODEX, SUNGROW, DOGE)) mosaic.add(id)
    mosaic.focus(CLAUDE)
}

val PORTRAIT = 390.dp to 844.dp
val LANDSCAPE = 844.dp to 390.dp
val DESKTOP = 1440.dp to 900.dp
