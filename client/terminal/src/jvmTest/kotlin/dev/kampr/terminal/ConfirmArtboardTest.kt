package dev.kampr.terminal

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.ImageComposeScene
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.platform.Font
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.theme.FamilyId
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.ThemeSpec
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.terminal.guard.SubmitGuard
import dev.kampr.terminal.input.Esc
import dev.kampr.terminal.input.InputSink
import dev.kampr.terminal.input.PaneKeyRow
import dev.kampr.terminal.view.TerminalView
import java.io.File
import kotlin.test.Test
import kotlin.test.assertNotNull

private const val PANE = "01JKAMPRNODE0000000000000/w3:p1"

private object ArtboardIo : PaneIo {
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String) = PanePrefs()
    override fun info(paneId: String) = PaneInfo(
        id = PANE, nodeId = "01JKAMPRNODE0000000000000", workspace = "kampr", tab = "1",
        cwd = "~/dev/kampr", agent = null, agentStatus = "idle", cols = 62, rows = 24,
        scrollbackRows = 900,
    )
}

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

private fun tokensFor(spec: ThemeSpec): KamprTokens {
    val fonts = KamprFonts(family(spec.ui), family(spec.mono), family(FamilyId.JetBrainsMono))
    return KamprTokens(spec, fonts, typography(fonts, spec.label, TypeScale.Phone))
}

private val SESSION_LINES = listOf(
    "dbrain@comingclean ~/dev/kampr \$ cargo test -p kampr-term",
    "",
    "   Compiling kampr-term v0.1.0",
    "    Finished `test` profile in 3.41s",
    "     Running unittests src/lib.rs",
    "",
    "test result: ok. 9 passed; 0 failed",
    "",
    "dbrain@comingclean ~/dev/kampr \$ du -sh build",
    "1.9G    build",
    "",
    "dbrain@comingclean ~/dev/kampr \$ rm -rf build",
)

private fun shellPane(): PaneState {
    val pane = PaneState(PANE, StyleTable())
    val rows = 24
    val top = rows - SESSION_LINES.size
    pane.applyReset(
        ServerMsg.GridReset(
            pane = PANE,
            cols = 62,
            rows = rows,
            rowsData = SESSION_LINES.mapIndexedNotNull { index, text ->
                text.takeIf { it.isNotEmpty() }?.let { RowDiff(top + index, listOf(Run(0, it))) }
            },
            cursor = Cursor(SESSION_LINES.last().length, rows - 1, true),
            links = emptyList(),
        ),
    )
    // A shell pane with a ring fills width and pans, which is the geometry an operator actually
    // reads a command line at; fill-height is for alt-screen panes with nothing above the grid.
    pane.applyScrollback(
        ServerMsg.Scrollback(
            pane = PANE,
            fromTop = 0,
            rows = (0 until 60).map { RowDiff(it, listOf(Run(0, HISTORY[it % HISTORY.size]))) },
            totalRows = 60,
            complete = false,
            capped = true,
        ),
    )
    return pane
}

private val HISTORY = listOf(
    "   Compiling kampr-core v0.1.0 (crates/kampr-core)",
    "   Compiling kampr-node v0.1.0 (crates/kampr-node)",
    "    Finished `dev` profile [unoptimized] in 11.2s",
    "     Running `target/debug/kampr serve`",
    "",
)

private class ArtboardSurfaces(private val session: PaneSession) : PaneSurfaces {
    @Composable
    override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) =
        TerminalView(pane, session, ArtboardIo, modifier)

    @Composable
    override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Unit

    @Composable
    override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) {
        val sink = InputSink(pane.id, ArtboardIo, session.latches)
        PaneKeyRow(
            session, sink, compact, enabled = true,
            modifier = modifier.onSizeChanged { session.keyRowHeight = it.height.toFloat() },
        )
    }
}

class ConfirmArtboardTest {
    @Test
    fun theConfirmSheetRendersOverALiveShellPane() {
        val pane = shellPane()
        val session = PaneSession(PANE)
        val guard = SubmitGuard(pane, ArtboardIo, session.confirm)
        InputSink(PANE, ArtboardIo, session.latches, guard).raw(Esc.ENTER)
        assertNotNull(session.confirm.held, "the artboard has to be of a real hold, not a fake one")

        val density = Density(2f)
        val scene = ImageComposeScene(
            width = with(density) { 390.dp.roundToPx() },
            height = with(density) { 844.dp.roundToPx() },
            density = density,
        ) {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme),
                LocalPaneIo provides ArtboardIo,
            ) {
                Box(Modifier.fillMaxSize()) {
                    PaneScreenMobile(
                        pane = pane,
                        info = ArtboardIo.info(PANE),
                        view = PaneView.Terminal,
                        surfaces = ArtboardSurfaces(session),
                        landscape = false,
                        readOnly = false,
                        onBack = {},
                        onView = {},
                        onAnswer = {},
                    )
                }
            }
        }
        try {
            scene.render()
            val image = scene.render()
            val file = File("build/artboards/destructive-confirm.png")
            file.parentFile.mkdirs()
            file.writeBytes(requireNotNull(image.encodeToData()).bytes)
        } finally {
            scene.close()
        }
    }
}
