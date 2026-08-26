package dev.kampr.terminal

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.remember
import androidx.compose.ui.ImageComposeScene
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.platform.Font
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.runtime.CompositionLocalProvider
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
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.theme.AllThemes
import dev.kampr.shared.theme.TerminalPalette
import dev.kampr.shared.theme.terminalSkin
import dev.kampr.shared.wire.ColorSpec
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Style
import dev.kampr.terminal.render.GridRenderer
import dev.kampr.terminal.render.ModeSelector
import dev.kampr.terminal.render.ResolvedStyles
import dev.kampr.terminal.render.SurfaceRows
import dev.kampr.terminal.render.TextCache
import java.io.File
import kotlin.test.Test
import kotlin.test.assertTrue


private fun rgb(hex: Int) = ColorSpec.Rgb((hex shr 16) and 0xFF, (hex shr 8) and 0xFF, hex and 0xFF)

// Real dark-authored agent output: Claude's own truecolour (#f6e2b7 at L 0.774, #abdfa7 at
// L 0.642) beside the indexed slots codex draws its chrome from, plus a diff and a faint rule.
private val styles = listOf(
    Style(),
    Style(fg = rgb(0xF6E2B7)),
    Style(fg = rgb(0xABDFA7)),
    Style(fg = rgb(0x7F7F7F), dim = true),
    Style(fg = ColorSpec.Indexed(4), bold = true),
    Style(fg = ColorSpec.Indexed(2)),
    Style(fg = ColorSpec.Indexed(1)),
    Style(fg = ColorSpec.Indexed(3)),
    Style(fg = ColorSpec.Indexed(6)),
    Style(fg = ColorSpec.Indexed(5)),
    Style(fg = ColorSpec.Indexed(8)),
    Style(fg = ColorSpec.Indexed(12)),
    Style(fg = ColorSpec.Indexed(15), bg = ColorSpec.Indexed(4)),
    Style(fg = ColorSpec.Indexed(2), bg = rgb(0x0B2A18)),
    Style(fg = ColorSpec.Indexed(1), bg = rgb(0x2A0F0F)),
    Style(fg = rgb(0xD787D7), italic = true),
    Style(fg = ColorSpec.Indexed(7), underline = true),
)

private fun row(index: Int, vararg cells: Pair<Int, String>) =
    RowDiff(index, cells.map { Run(it.first, it.second) })

private val transcript = listOf(
    row(0, 4 to "✻ Welcome to Claude Code", 0 to "  ", 3 to "v2.4.1"),
    row(1),
    row(2, 1 to "● ", 1 to "Read", 0 to "(crates/kampr-term/src/grid.rs)"),
    row(3, 10 to "  ⎿  read 412 lines "),
    row(4),
    row(5, 2 to "● Update(", 16 to "crates/kampr-node/src/scrollback.rs", 2 to ")"),
    row(6, 13 to "  +   let zoom = fit_w.max(fit_h);           "),
    row(7, 14 to "  -   let zoom = fit_w.min(fit_h);           "),
    row(8, 10 to "  ⎿  1 addition, 1 removal"),
    row(9),
    row(10, 5 to "✓ ", 0 to "cargo test", 3 to " · ", 5 to "142 passed", 3 to " · ", 6 to "1 failed"),
    row(11, 6 to "  error[E0308]", 0 to ": mismatched types"),
    row(12, 11 to "   --> crates/kampr-term/src/grid.rs:88:17"),
    row(13, 7 to "  warning", 0 to ": unused variable ", 8 to "`rows`"),
    row(14),
    row(15, 9 to "◆ ", 0 to "Do you want to make this edit?"),
    row(16, 12 to " 1. Yes ", 0 to "  2. Yes, allow always   3. No"),
    row(17),
    row(18, 3 to "  ? for shortcuts                              ", 3 to "⏵⏵ accept edits on"),
    row(19, 0 to "> ", 3 to "_"),
)

private fun pane(): PaneState {
    val state = PaneState("n/w1:p1", StyleTable())
    state.styles.append(0, styles)
    state.applyReset(
        ServerMsg.GridReset(
            pane = state.id,
            cols = 78,
            rows = transcript.size,
            rowsData = transcript,
            cursor = Cursor(2, transcript.size - 1, true),
            links = emptyList(),
        ),
    )
    return state
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


private object NoIo : PaneIo {
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String): PanePrefs = PanePrefs()
}

private fun uiFamily(id: FamilyId): FontFamily {
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

private fun artboardTokens(spec: ThemeSpec, ground: Ground, terminal: FontFamily): KamprTokens {
    val grounded = spec.on(ground)
    val fonts = KamprFonts(uiFamily(grounded.ui), uiFamily(grounded.mono), terminal)
    return KamprTokens(grounded, fonts, typography(fonts, grounded.label, TypeScale.Phone))
}

class AnsiArtboardTest {
    // ADR 0009: the terminal keeps a dark ground under both app grounds, so this sheet is the
    // proof — the same pane, once per theme, and the light column must be identical to the dark.
    @Test
    fun agentOutputRendersUnderEveryThemeSkin() {
        val state = pane()
        val rows = SurfaceRows(state)
        val family = terminalFamily()
        val density = Density(2f)
        val width = 620.dp
        val height = 300.dp
        val scene = ImageComposeScene(
            width = with(density) { (width * AllThemes.size).roundToPx() },
            height = with(density) { height.roundToPx() },
            density = density,
        ) {
            val measurer = rememberTextMeasurer(cacheSize = 0)
            Row(Modifier.fillMaxSize()) {
                for (spec in AllThemes) {
                    val palette = remember(spec) { TerminalPalette(terminalSkin(spec.id)) }
                    val cache = remember(spec) { TextCache(measurer, family) }
                    val renderer = remember(spec) { GridRenderer(cache, ModeSelector()) }
                    val resolved = remember(spec) { ResolvedStyles(palette) }
                    Box(Modifier.width(width).height(height)) {
                        Canvas(Modifier.fillMaxSize()) {
                            resolved.sync(state.styles)
                            val metrics = cache.metrics(13.sp)
                            renderer.draw(
                                scope = this,
                                rows = rows,
                                styles = resolved,
                                cellWidth = metrics.width,
                                cellHeight = metrics.height,
                                originX = 8f,
                                originY = 8f,
                                cursorCol = 2,
                                cursorRow = transcript.size - 1,
                                cursorOn = true,
                                selection = null,
                                selectionWash = palette.selectionWash,
                                linkInk = palette.linkInk,
                            )
                        }
                    }
                }
            }
        }
        try {
            scene.render()
            val image = scene.render()
            OUT.mkdirs()
            File(OUT, "ansi-skins.png").writeBytes(requireNotNull(image.encodeToData()).bytes)
            assertTrue(image.width > 0)
        } finally {
            scene.close()
        }
    }

    // The light-ground proof: the same pane, chrome in light, terminal dark inside it. If the
    // seam ever reads as breakage rather than as a device, this is the artboard that shows it.
    @Test
    fun paneScreenHoldsADarkTerminalUnderALightGround() {
        val state = pane()
        val info = PaneInfo(
            id = state.id, nodeId = "01JNODE", workspace = "kampr", tab = "1",
            cwd = "/home/dbrain/dev/kampr", agent = "claude", agentStatus = "blocked",
            cols = 78, rows = 20, scrollbackRows = 412,
        )
        val family = terminalFamily()
        val density = Density(2f)
        val width = 390.dp
        val height = 844.dp
        for (ground in Ground.entries) {
            val scene = ImageComposeScene(
                width = with(density) { (width * AllThemes.size).roundToPx() },
                height = with(density) { height.roundToPx() },
                density = density,
            ) {
                CompositionLocalProvider(LocalPaneIo provides NoIo, LocalGround provides ground) {
                    Row(Modifier.fillMaxSize()) {
                        for (spec in AllThemes) {
                            CompositionLocalProvider(
                                LocalTokens provides artboardTokens(spec, ground, family),
                            ) {
                                Box(Modifier.width(width).height(height)) {
                                    PaneScreenMobile(
                                        pane = state,
                                        info = info,
                                        view = PaneView.Terminal,
                                        surfaces = TerminalSurfaces(),
                                        landscape = false,
                                        readOnly = false,
                                        onBack = {},
                                        onView = {},
                                        onAnswer = {},
                                    )
                                }
                            }
                        }
                    }
                }
            }
            try {
                repeat(3) { scene.render() }
                val image = scene.render()
                OUT.mkdirs()
                File(OUT, "pane-terminal-${ground.name.lowercase()}.png")
                    .writeBytes(requireNotNull(image.encodeToData()).bytes)
                assertTrue(image.width > 0)
            } finally {
                scene.close()
            }
        }
    }
}
