package dev.kampr.conversation

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
import dev.kampr.shared.model.PaneState
import androidx.compose.ui.text.font.FontStyle
import dev.kampr.shared.theme.FamilyId
import dev.kampr.shared.theme.GlyphGaps
import dev.kampr.shared.theme.gapsOfForTest
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.ThemeSpec
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.on
import dev.kampr.shared.theme.typography
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.ui.LocalConnectionStatus
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import org.jetbrains.skia.Image
import java.io.File

const val PANE_ID = "01JNODE.../w3:p2"

object RecordingIo : PaneIo {
    val sent = mutableListOf<ClientMsg>()
    override fun send(msg: ClientMsg) {
        sent += msg
    }
    override fun prefs(paneId: String): PanePrefs = PanePrefs()
    override fun show(view: PaneView) = Unit
}

fun demoPane(vararg frames: String): Pair<KamprStore, PaneState> {
    val store = KamprStore()
    for (frame in frames) store.accept(requireNotNull(Wire.decode(frame)) { "undecodable frame" })
    store.accept(
        ServerMsg.Pending(
            pane = PANE_ID,
            question = "Do you want to make this edit?",
            options = listOf(
                dev.kampr.shared.wire.PendingOption("1", "Yes"),
                dev.kampr.shared.wire.PendingOption("2", "Always"),
                dev.kampr.shared.wire.PendingOption("3", "No"),
            ),
            source = "screen",
        )
    )
    return store to store.pane(PANE_ID)
}

fun runPane(): PaneState {
    val store = KamprStore()
    store.accept(ServerMsg.Convo(pane = PANE_ID, cursor = "r-1", more = false, turns = TOOL_RUN_TURNS))
    return store.pane(PANE_ID)
}

fun demoInfo(
    agent: String? = "claude",
    conversation: Boolean = true,
    status: String = "blocked",
    converses: Boolean = false,
) = PaneInfo(
    id = PANE_ID,
    nodeId = "01JNODE",
    workspace = "kampr",
    tab = "1",
    cwd = "/home/dbrain/dev/kampr",
    agent = agent,
    agentStatus = status,
    cols = 74,
    rows = 30,
    hasConversation = conversation,
    converses = converses,
)

// Compose resources resolve fonts asynchronously, which a headless render never waits for, so
// the artboards load the same .ttf files straight off disk and skip the gate.
private fun family(id: FamilyId): FontFamily {
    val dir = File("../shared/src/commonMain/composeResources/font")
    fun face(name: String, weight: FontWeight, style: FontStyle = FontStyle.Normal) =
        Font(name, File(dir, "$name.ttf").readBytes(), weight, style)
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

fun tokensFor(
    spec: ThemeSpec,
    scale: TypeScale,
    ground: Ground = Ground.Dark,
    // The gap tables the app runs with. Defaulted on rather than off because an artboard that
    // draws prose differently from the app is an artboard that cannot show the app being wrong —
    // and a test that wants the *un*routed rendering asks for it, which is what proves the seam.
    routed: Boolean = true,
): KamprTokens {
    val grounded = spec.on(ground)
    // The real terminal face, not JetBrains Mono. It is what ships, it is the only face carrying
    // the symbols prose gets routed to, and nothing outside the terminal module reads it otherwise.
    val fonts = KamprFonts(
        family(grounded.ui),
        family(grounded.mono),
        terminalFamily(),
        if (routed) gapsOfForTest(grounded.ui) else GlyphGaps.none,
        if (routed) gapsOfForTest(grounded.mono) else GlyphGaps.none,
    )
    return KamprTokens(grounded, fonts, typography(fonts, grounded.label, scale))
}

private fun terminalFamily(): FontFamily {
    val dir = File("../shared/src/commonMain/composeResources/font")
    fun face(name: String, weight: FontWeight, style: FontStyle = FontStyle.Normal) =
        Font(name, File(dir, "$name.ttf").readBytes(), weight, style)
    return FontFamily(
        face("terminalmono_regular", FontWeight.W400),
        face("terminalmono_bold", FontWeight.W700),
        face("terminalmono_italic", FontWeight.W400, FontStyle.Italic),
        face("terminalmono_bolditalic", FontWeight.W700, FontStyle.Italic),
    )
}

fun <T> withScene(
    width: Dp,
    height: Dp,
    spec: ThemeSpec,
    scale: TypeScale,
    content: @Composable () -> Unit,
    body: (ImageComposeScene) -> T,
): T = withScene(width, height, spec, scale, Ground.Dark, content, body)

fun <T> withScene(
    width: Dp,
    height: Dp,
    spec: ThemeSpec,
    scale: TypeScale,
    ground: Ground,
    content: @Composable () -> Unit,
    body: (ImageComposeScene) -> T,
): T {
    val density = Density(2f)
    val scene = ImageComposeScene(
        width = with(density) { width.roundToPx() },
        height = with(density) { height.roundToPx() },
        density = density,
    ) {
        CompositionLocalProvider(
            LocalTokens provides tokensFor(spec, scale, ground),
            LocalPaneIo provides RecordingIo,
            LocalConnectionStatus provides ConnectionStatus.Live("full"),
        ) {
            Box(Modifier.fillMaxSize()) { content() }
        }
    }
    return try {
        body(scene)
    } finally {
        scene.close()
    }
}

fun renderArtboard(
    width: Dp,
    height: Dp,
    spec: ThemeSpec,
    scale: TypeScale,
    file: File,
    ground: Ground = Ground.Dark,
    content: @Composable () -> Unit,
): Image = withScene(width, height, spec, scale, ground, content) { scene ->
    scene.render()
    val image = scene.render()
    file.parentFile.mkdirs()
    file.writeBytes(requireNotNull(image.encodeToData()).bytes)
    image
}

val PORTRAIT = 390.dp to 844.dp
val LANDSCAPE = 844.dp to 390.dp
val DESKTOP = 1440.dp to 900.dp
