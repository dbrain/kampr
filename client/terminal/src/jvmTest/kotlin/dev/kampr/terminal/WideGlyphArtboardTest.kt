package dev.kampr.terminal

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
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
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.theme.AllThemes
import dev.kampr.shared.theme.TerminalPalette
import dev.kampr.shared.theme.terminalSkin
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.terminal.render.GridRenderer
import dev.kampr.terminal.render.ModeSelector
import dev.kampr.terminal.render.RenderMode
import dev.kampr.terminal.render.ResolvedStyles
import dev.kampr.terminal.render.SurfaceRows
import dev.kampr.terminal.render.TextCache
import java.awt.image.BufferedImage
import java.io.ByteArrayInputStream
import java.io.File
import javax.imageio.ImageIO
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val COLS = 16

private fun paneOf(vararg rows: List<Run>): PaneState {
    val state = PaneState("n/w1:p1", StyleTable())
    state.applyReset(
        ServerMsg.GridReset(
            pane = state.id,
            cols = COLS,
            rows = rows.size,
            rowsData = rows.mapIndexed { index, runs -> RowDiff(index, runs) },
            cursor = Cursor(0, 0, false),
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

private fun render(rows: List<List<Run>>, name: String): Pair<BufferedImage, Float> {
    val state = paneOf(*rows.toTypedArray())
    val surface = SurfaceRows(state)
    val family = terminalFamily()
    val density = Density(2f)
    var pitch = 0f
    val scene = ImageComposeScene(
        width = with(density) { 320.dp.roundToPx() },
        height = with(density) { 60.dp.roundToPx() },
        density = density,
    ) {
        val measurer = rememberTextMeasurer(cacheSize = 0)
        val palette = remember { TerminalPalette(terminalSkin(AllThemes.first().id)) }
        val cache = remember { TextCache(measurer, family) }
        val renderer = remember { GridRenderer(cache, ModeSelector().also { it.forced = RenderMode.CachedRuns }) }
        val resolved = remember { ResolvedStyles(palette) }
        Column(Modifier.fillMaxSize()) {
            Box(Modifier.width(320.dp).height(60.dp)) {
                Canvas(Modifier.fillMaxSize()) {
                    resolved.sync(state.styles)
                    val metrics = cache.metrics(16.sp)
                    pitch = metrics.width
                    renderer.draw(
                        scope = this,
                        rows = surface,
                        styles = resolved,
                        cellWidth = metrics.width,
                        cellHeight = metrics.height,
                        originX = 0f,
                        originY = 0f,
                        cursorCol = 0,
                        cursorRow = 0,
                        cursorOn = false,
                        selection = null,
                        selectionWash = palette.selectionWash,
                        linkInk = palette.linkInk,
                    )
                }
            }
        }
    }
    return try {
        scene.render()
        val image = scene.render()
        OUT.mkdirs()
        val bytes = requireNotNull(image.encodeToData()).bytes
        File(OUT, "$name.png").writeBytes(bytes)
        ImageIO.read(ByteArrayInputStream(bytes)) to pitch
    } finally {
        scene.close()
    }
}

// Probe #210 downstream of the emulator: each wide glyph owns two columns and everything after it
// keeps the pitch. The assertion is on which cells hold ink, not on what the ink looks like — a
// fallback face may draw a CJK glyph or an emoji at any width, but the ASCII either side of it has
// to sit on the grid. Before the fix this row read `AB\u65e5 \u672c \u8a9e CD` and its CD was in
// columns 10 and 11.
class WideGlyphArtboardTest {
    private fun inked(image: BufferedImage, pitch: Float): List<Boolean> {
        val ground = image.getRGB(image.width - 1, image.height - 1)
        return (0 until COLS).map { col ->
            val from = (col * pitch).toInt()
            val to = minOf(((col + 1) * pitch).toInt(), image.width)
            (from until to).any { x -> (0 until image.height).any { image.getRGB(x, it) != ground } }
        }
    }

    private fun firstInk(image: BufferedImage, col: Int, pitch: Float): Int {
        val ground = image.getRGB(image.width - 1, image.height - 1)
        val from = (col * pitch).toInt()
        for (x in from until minOf(((col + 1) * pitch).toInt(), image.width)) {
            if ((0 until image.height).any { image.getRGB(x, it) != ground }) return x - from
        }
        return Int.MAX_VALUE
    }

    private fun check(runs: List<Run>, name: String, glyphCols: IntRange, tail: IntRange) {
        val (image, pitch) = render(listOf(runs), name)
        assertTrue(pitch > 1f, "the cell pitch has to be a real measurement")
        val ink = inked(image, pitch)
        for (col in 0..1) assertTrue(ink[col], "$name: column $col is blank")
        assertTrue(glyphCols.any { ink[it] }, "$name: nothing was drawn in the wide glyphs' columns")
        for (col in tail) assertTrue(ink[col], "$name: column $col should carry the text after the glyphs")
        for (col in (tail.last + 1) until COLS) {
            assertTrue(!ink[col], "$name: column $col holds ink, so the row was pushed right")
        }
        assertTrue(
            firstInk(image, tail.first, pitch) <= 5,
            "$name: the text after the wide glyphs does not start at column ${tail.first}",
        )
    }

    @Test
    fun wideGlyphsSpanTwoColumnsAndTheTextAfterThemKeepsThePitch() {
        check(
            listOf(Run(0, "AB"), Run(0, "\u65e5\u672c\u8a9e", w = 2), Run(0, "CD")),
            "wide-glyphs-cjk",
            glyphCols = 2..7,
            tail = 8..9,
        )
        check(
            listOf(Run(0, "XY"), Run(0, "\uD83D\uDE80", w = 2), Run(0, "ZW")),
            "wide-glyphs-astral",
            glyphCols = 2..3,
            tail = 4..5,
        )
    }

    // Probe #223: the mark has to reach the screen without buying itself a column. A cell wearing
    // one is drawn on its own for the same reason a wide glyph is (probe #214) — nothing promises
    // the face that draws the mark advances exactly zero — so the proof is that the columns after
    // it have not moved, and that the ink is not the ink of a bare base.
    @Test
    fun aMarkedCellKeepsItsSingleColumnAndTheTextAfterItKeepsThePitch() {
        check(
            listOf(Run(0, "AB"), Run(0, "ee", m = listOf("\u0301", "\u0302")), Run(0, "CD")),
            "marked-cells-latin",
            glyphCols = 2..3,
            tail = 4..5,
        )
        val (marked, pitch) = render(
            listOf(listOf(Run(0, "AB"), Run(0, "e", m = listOf("\u0301")))),
            "marked-cells-accent",
        )
        val (bare, _) = render(listOf(listOf(Run(0, "ABe"))), "marked-cells-bare")
        assertTrue(pitch > 1f)
        assertTrue(
            columnPixels(marked, 2, pitch) != columnPixels(bare, 2, pitch),
            "the accented e draws the same ink as a bare e, so the mark never arrived",
        )
    }

    private fun columnPixels(image: BufferedImage, col: Int, pitch: Float): List<Int> {
        val from = (col * pitch).toInt()
        val to = minOf(((col + 1) * pitch).toInt(), image.width)
        return (from until to).flatMap { x -> (0 until image.height).map { image.getRGB(x, it) } }
    }
}
