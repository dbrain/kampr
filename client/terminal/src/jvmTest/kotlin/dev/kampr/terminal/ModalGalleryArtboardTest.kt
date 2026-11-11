package dev.kampr.terminal

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.asSkiaBitmap
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.v2.runSkikoComposeUiTest
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.PaneScreenDesktop
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.SafeArea
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.terminal.guard.SubmitGuard
import dev.kampr.terminal.input.Esc
import dev.kampr.terminal.input.InputSink
import dev.kampr.terminal.input.PaneKeyRow
import dev.kampr.terminal.view.PaneSizing
import dev.kampr.terminal.view.ResizePanel
import dev.kampr.terminal.view.TerminalView
import org.jetbrains.skia.EncodedImageFormat
import org.jetbrains.skia.Image
import java.io.File
import kotlin.test.Test
import kotlin.test.assertNotNull

private const val GALLERY_PANE = "01JHUB/w3:p3"

private val GALLERY_OUT = File("build/artboards/gallery")

private enum class Frame(val width: Int, val height: Int, val density: Float, val scale: TypeScale, val bars: SafeArea) {
    Portrait(390, 844, 2.625f, TypeScale.Phone, SafeArea(top = 44.dp, bottom = 46.dp)),
    Desktop(1280, 800, 1.5f, TypeScale.Desk, SafeArea(0.dp, 0.dp)),
}

private object GalleryIo : PaneIo {
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String) = PanePrefs(mapOf("confirm" to "on"))
    override fun info(paneId: String) = PaneInfo(
        id = GALLERY_PANE, nodeId = "01JHUB", workspace = "kampr", tab = "1",
        cwd = "~/dev/kampr", agent = null, agentStatus = "idle", cols = 94, rows = 30,
        scrollbackRows = 900,
    )
}

private val LINES = listOf(
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
    val pane = PaneState(GALLERY_PANE, StyleTable())
    val rows = 30
    val top = rows - LINES.size
    pane.applyReset(
        ServerMsg.GridReset(
            pane = GALLERY_PANE,
            cols = 94,
            rows = rows,
            rowsData = LINES.mapIndexedNotNull { i, text -> text.takeIf { it.isNotEmpty() }?.let { RowDiff(top + i, listOf(Run(0, it))) } },
            cursor = Cursor(LINES.last().length, rows - 1, true),
            links = emptyList(),
        ),
    )
    pane.applyScrollback(
        ServerMsg.Scrollback(
            pane = GALLERY_PANE,
            fromTop = 0,
            rows = (0 until 60).map { RowDiff(it, listOf(Run(0, "   Compiling kampr-core v0.1.0 (crates/kampr-core) #$it"))) },
            totalRows = 60,
            complete = false,
            capped = true,
        ),
    )
    return pane
}

private class GallerySurfaces(private val session: PaneSession) : PaneSurfaces {
    @Composable
    override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = TerminalView(pane, session, GalleryIo, modifier)

    @Composable
    override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Unit

    @Composable
    override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) {
        PaneKeyRow(
            session,
            InputSink(pane.id, GalleryIo, session.latches),
            compact,
            enabled = true,
            modifier = modifier.onSizeChanged { session.keyRowHeight = it.height.toFloat() },
        )
    }

    @Composable
    override fun Zoom(pane: PaneState, modifier: Modifier) = dev.kampr.terminal.view.ZoomButton(session, modifier)
}

@OptIn(ExperimentalTestApi::class)
private fun shoot(name: String, frame: Frame, ground: Ground = Ground.Dark, content: @Composable () -> Unit) {
    val file = File(GALLERY_OUT, "$name-${frame.name.lowercase()}${if (ground == Ground.Light) "-light" else ""}.png")
    runSkikoComposeUiTest(Size(frame.width * frame.density, frame.height * frame.density), Density(frame.density)) {
        mainClock.autoAdvance = false
        setContent {
            CompositionLocalProvider(
                dev.kampr.shared.theme.LocalTokens provides tokensFor(SoftTheme, frame.scale, ground),
                LocalSafeArea provides frame.bars,
                LocalPaneIo provides GalleryIo,
            ) {
                Box(Modifier.fillMaxSize()) { content() }
            }
        }
        mainClock.advanceTimeBy(800)
        file.parentFile.mkdirs()
        file.writeBytes(requireNotNull(Image.makeFromBitmap(captureToImage().asSkiaBitmap()).encodeToData(EncodedImageFormat.PNG)).bytes)
    }
}

@Composable
private fun Screen(frame: Frame, pane: PaneState, session: PaneSession) {
    when (frame) {
        Frame.Portrait -> PaneScreenMobile(
            pane = pane,
            info = GalleryIo.info(GALLERY_PANE),
            view = PaneView.Terminal,
            surfaces = GallerySurfaces(session),
            landscape = false,
            readOnly = false,
            onBack = {},
            onView = {},
        )

        Frame.Desktop -> PaneScreenDesktop(
            pane = pane,
            info = GalleryIo.info(GALLERY_PANE),
            view = PaneView.Terminal,
            surfaces = GallerySurfaces(session),
            readOnly = false,
            onView = {},
        )
    }
}

private fun held(): Pair<PaneState, PaneSession> {
    val pane = shellPane()
    val session = PaneSession(GALLERY_PANE)
    InputSink(GALLERY_PANE, GalleryIo, session.latches, SubmitGuard(pane, GalleryIo, session.confirm)).raw(Esc.ENTER)
    assertNotNull(session.confirm.held, "the artboard has to be of a real hold")
    return pane to session
}

class ModalGalleryArtboardTest {
    @Test
    fun theTerminalsOwnOverlaysRender() {
        for (frame in Frame.entries) {
            val zoomed = PaneSession(GALLERY_PANE).also { it.view.sheetOpen = true }
            val zoomPane = shellPane()
            shoot("zoom-sheet", frame) { Screen(frame, zoomPane, zoomed) }
            val (confirmPane, confirming) = held()
            shoot("confirm-sheet", frame) { Screen(frame, confirmPane, confirming) }
            val menu = PaneSession(GALLERY_PANE).also {
                it.view.menuAt = Offset(frame.width * frame.density * 0.4f, frame.height * frame.density * 0.45f)
            }
            val menuPane = shellPane()
            shoot("grid-menu", frame) { Screen(frame, menuPane, menu) }
            shoot("resize-panel", frame) {
                Box(Modifier.fillMaxSize().background(Kampr.tokens.color.bg), contentAlignment = Alignment.Center) {
                    Box(
                        Modifier.width(360.dp)
                            .background(Kampr.tokens.color.bar, RoundedCornerShape(Kampr.tokens.radii.lg))
                            .padding(16.dp),
                    ) {
                        ResizePanel(PaneSizing(94, 30, 52, 38, held = false, canMatch = true), { _, _ -> }, {})
                    }
                }
            }
        }
        val (pane, session) = held()
        shoot("confirm-sheet", Frame.Portrait, Ground.Light) { Screen(Frame.Portrait, pane, session) }
    }
}
