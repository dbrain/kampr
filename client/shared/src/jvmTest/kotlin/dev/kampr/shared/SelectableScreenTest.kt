package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.text.selection.LocalTextSelectionColors
import androidx.compose.foundation.text.selection.TextSelectionColors
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.ImageComposeScene
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.input.pointer.PointerType
import androidx.compose.ui.platform.ClipEntry
import androidx.compose.ui.platform.Clipboard
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.platform.LocalTextToolbar
import androidx.compose.ui.platform.TextToolbar
import androidx.compose.ui.platform.TextToolbarStatus
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.ConnectPanel
import dev.kampr.shared.ui.ErrorStrip
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.PhoneScaffold
import dev.kampr.shared.ui.Screen
import java.awt.datatransfer.DataFlavor
import java.awt.datatransfer.Transferable
import java.io.ByteArrayInputStream
import javax.imageio.ImageIO
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private val HIGHLIGHT = Color(0xFFFF00FF)

private const val REFUSAL = "This node does not know this device."

private class SelectionToolbar : TextToolbar {
    var copy: (() -> Unit)? = null
    override val status: TextToolbarStatus get() = TextToolbarStatus.Hidden
    override fun hide() = Unit
    override fun showMenu(
        rect: Rect,
        onCopyRequested: (() -> Unit)?,
        onPasteRequested: (() -> Unit)?,
        onCutRequested: (() -> Unit)?,
        onSelectAllRequested: (() -> Unit)?,
    ) {
        copy = onCopyRequested
    }
}

@OptIn(ExperimentalComposeUiApi::class)
private class HeldClipboard : Clipboard {
    var entry: ClipEntry? = null
    override val nativeClipboard: Any get() = throw UnsupportedOperationException()
    override suspend fun getClipEntry(): ClipEntry? = entry
    override suspend fun setClipEntry(clipEntry: ClipEntry?) { entry = clipEntry }

    val pasted: String?
        get() = (entry?.nativeClipEntry as? Transferable)?.getTransferData(DataFlavor.stringFlavor) as String?
}

@Composable
private fun Panel() {
    ConnectPanel(current = null, error = REFUSAL, onConnect = {}, onScan = {})
}

// The app's own scaffold, given the app's own screens: what makes a screen selectable is where the
// scaffold puts it, so a harness that mounted the panel by itself would prove nothing.
@Composable
private fun Scaffolded(screen: Screen) {
    Bars {
        PhoneScaffold(Breakpoint.Portrait, screen, BARS, {}) { Box(Modifier.fillMaxSize()) { Panel() } }
    }
}

private fun <T> scene(content: @Composable () -> Unit, body: (ImageComposeScene) -> T): T {
    val density = Density(2f)
    val made = ImageComposeScene(
        width = with(density) { 390.dp.roundToPx() },
        height = with(density) { 844.dp.roundToPx() },
        density = density,
    ) { content() }
    return try {
        body(made)
    } finally {
        made.close()
    }
}

private fun highlighted(scene: ImageComposeScene): Int {
    val png = requireNotNull(scene.render().encodeToData()).bytes
    val image = ImageIO.read(ByteArrayInputStream(png))
    val want = ((HIGHLIGHT.red * 255).toInt() shl 16) or
        ((HIGHLIGHT.green * 255).toInt() shl 8) or (HIGHLIGHT.blue * 255).toInt()
    var found = 0
    for (y in 0 until image.height) for (x in 0 until image.width) {
        if (image.getRGB(x, y) and 0xFFFFFF == want) found++
    }
    return found
}

// A long press is what starts a selection under a finger, and it is a real wall-clock wait: the
// gesture is timed against the scene's clock, which only a rendered frame advances.
private fun ImageComposeScene.longPressDrag(from: Offset, to: Offset) {
    repeat(3) { render() }
    var at = 0L
    fun touch(kind: PointerEventType, where: Offset) {
        sendPointerEvent(kind, where, timeMillis = at, type = PointerType.Touch)
        at += 50
    }
    touch(PointerEventType.Press, from)
    Thread.sleep(900)
    at = 900
    render()
    touch(PointerEventType.Move, Offset((from.x + to.x) / 2, (from.y + to.y) / 2))
    touch(PointerEventType.Move, to)
    touch(PointerEventType.Release, to)
    render()
}

private val PANEL = Offset(60f, 180f) to Offset(720f, 1400f)

private fun highlightOn(screen: Screen): Int = scene(
    content = {
        CompositionLocalProvider(LocalTextSelectionColors provides TextSelectionColors(HIGHLIGHT, HIGHLIGHT)) {
            Scaffolded(screen)
        }
    },
    body = { made ->
        made.longPressDrag(PANEL.first, PANEL.second)
        highlighted(made)
    },
)

@OptIn(ExperimentalComposeUiApi::class)
private fun copiedFrom(drag: Pair<Offset, Offset>, content: @Composable () -> Unit): String? {
    val toolbar = SelectionToolbar()
    val clipboard = HeldClipboard()
    scene(
        content = {
            CompositionLocalProvider(
                LocalTextToolbar provides toolbar,
                LocalClipboard provides clipboard,
                content = content,
            )
        },
        body = { made ->
            made.longPressDrag(drag.first, drag.second)
            toolbar.copy?.invoke()
            repeat(4) { made.render(); Thread.sleep(60) }
        },
    )
    return clipboard.pasted
}

class SelectableScreenTest {
    @Test
    fun aNodesOwnRefusalOnASettingsScreenCanBeDraggedAcrossAndSelected() {
        assertTrue(highlightOn(Screen.Setup) > 0, "dragging across the screen highlighted nothing")
    }

    // The terminal draws its grid on a canvas and runs its own selection off gestures — anchor,
    // head, block mode — that a container above it would take first. So the scaffold hands a pane
    // its body untouched, and the transcript inside a pane wraps its own.
    @Test
    fun aPaneIsLeftToTheSelectionItAlreadyHas() {
        assertEquals(
            0,
            highlightOn(Screen.Pane("01JNODE.../w3:p2", PaneView.Terminal)),
            "the scaffold put a selection over a pane",
        )
    }

    @Test
    fun whatIsCopiedIsWhatTheNodeSaidAndNotTheChromeAroundIt() {
        val pasted = copiedFrom(PANEL) { Scaffolded(Screen.Setup) }
        assertTrue(pasted != null, "copying across the screen produced nothing")
        assertTrue(REFUSAL in pasted!!, "the node's own refusal was not copied: $pasted")
        assertTrue("kampr init" in pasted, "the screen's own prose was not copied: $pasted")
        assertTrue("192.168.1.24:8790" !in pasted, "an empty field's hint was copied: $pasted")
        assertTrue("Scan a pairing code" !in pasted, "a button's caption was copied: $pasted")
        assertTrue("Connect" !in pasted, "a button's caption was copied: $pasted")
    }

    // A strip floats over every screen rather than inside one, so it is outside the body that
    // makes a screen selectable, and it carries the sentence a reader is most likely to quote.
    @Test
    fun theWordsAStripCarriesCanBeSelectedAndCopied() {
        val pasted = copiedFrom(Offset(100f, 130f) to Offset(600f, 200f)) {
            Bars { Box(Modifier.fillMaxSize()) { ErrorStrip(REFUSAL, "refused", {}) } }
        }
        assertTrue(pasted != null && REFUSAL in pasted, "copying a strip produced ${pasted ?: "nothing"}")
    }

    // Tapping a strip is the only way to dismiss one, and a selection over the same words claims
    // the same pointer.
    @OptIn(ExperimentalTestApi::class)
    @Test
    fun aStripStillDismissesOnATap() = runComposeUiTest {
        var dismissed = false
        setContent { Bars { Box(Modifier.fillMaxSize()) { ErrorStrip(REFUSAL, "refused", { dismissed = true }) } } }
        onNodeWithContentDescription(REFUSAL, substring = true).performClick()
        assertTrue(dismissed, "the strip under the selection no longer dismisses")
    }

    @OptIn(ExperimentalTestApi::class)
    @Test
    fun aButtonOnASelectableScreenStillPresses() = runComposeUiTest {
        var scanned = false
        setContent {
            Bars {
                PhoneScaffold(Breakpoint.Portrait, Screen.Setup, BARS, {}) {
                    Box(Modifier.fillMaxSize()) {
                        ConnectPanel(current = null, error = REFUSAL, onConnect = {}, onScan = { scanned = true })
                    }
                }
            }
        }
        onNodeWithContentDescription("Scan a pairing code with the camera").performClick()
        assertTrue(scanned, "the button under the selection never fired")
    }
}
