package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.text.selection.LocalTextSelectionColors
import androidx.compose.foundation.text.selection.TextSelectionColors
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.ImageComposeScene
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.input.pointer.PointerType
import androidx.compose.ui.platform.ClipEntry
import androidx.compose.ui.platform.Clipboard
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.platform.LocalTextToolbar
import androidx.compose.ui.platform.TextToolbar
import androidx.compose.ui.platform.TextToolbarStatus
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasScrollAction
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.wire.Wire
import java.awt.datatransfer.DataFlavor
import java.awt.datatransfer.Transferable
import java.io.ByteArrayInputStream
import javax.imageio.ImageIO
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

// Loud on purpose and provided rather than themed: the count below is of the selection highlight
// and nothing else, so it has to be a colour the transcript never paints by itself.
private val HIGHLIGHT = Color(0xFFFF00FF)

private const val ANSWER =
    "{\"cursor\":\"a-1\",\"more\":false,\"pane\":\"01JNODE.../w3:p2\",\"t\":\"convo\",\"turns\":[" +
        "{\"at\":\"2026-08-20T09:00:01.000Z\",\"blocks\":[{\"b\":\"md\",\"text\":\"the letterbox came from " +
        "min where max was meant, and every pane on a phone showed it\"}],\"id\":\"a-1\",\"role\":\"assistant\"}]}"

private const val CARDS =
    "{\"cursor\":\"a-2\",\"more\":false,\"pane\":\"01JNODE.../w3:p2\",\"t\":\"convo\",\"turns\":[" +
        "{\"at\":\"2026-08-20T09:00:01.000Z\",\"blocks\":[" +
        "{\"b\":\"md\",\"text\":\"here:\\n\\n```bash\\nherdr pane list --json\\n```\\n\"}," +
        "{\"b\":\"md\",\"text\":\"[image · png]\",\"att\":{\"id\":\"att-7f3\",\"kind\":\"image\"," +
        "\"mime\":\"image/png\",\"bytes\":52831,\"name\":\"shot.png\"}}" +
        "],\"id\":\"a-2\",\"role\":\"assistant\"}]}"

internal fun paneOf(vararg frames: String): PaneState {
    val store = KamprStore()
    for (frame in frames) store.accept(requireNotNull(Wire.decode(frame)) { "undecodable frame" })
    return store.pane(PANE_ID)
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

private fun draggedAcrossTheTranscript(pane: PaneState): Int = withScene(
    PORTRAIT.first, PORTRAIT.second, SoftTheme, TypeScale.Phone,
    content = {
        CompositionLocalProvider(LocalTextSelectionColors provides TextSelectionColors(HIGHLIGHT, HIGHLIGHT)) {
            ConversationView(pane, demoInfo(), Modifier.fillMaxSize())
        }
    },
    body = { scene ->
        repeat(3) { scene.render() }
        // Measured against the rendered artboard, at density 2: the first line of the first step
        // sits at 120 dp on a 390x844 phone, inside the one box its whole reply is drawn as,
        // whose content starts 30 dp in. A press has to land in the *first* line of it: a long
        // press takes the word under it and the drag extends from there, so a press one line low
        // copies an answer that starts halfway through itself.
        scene.sendPointerEvent(PointerEventType.Press, Offset(70f, 255f))
        scene.sendPointerEvent(PointerEventType.Move, Offset(400f, 300f))
        scene.sendPointerEvent(PointerEventType.Move, Offset(700f, 340f))
        scene.sendPointerEvent(PointerEventType.Release, Offset(700f, 340f))
        highlighted(scene)
    },
)

// The two seams a copy actually passes through, both faked so the test can read what a reader
// would have pasted: the toolbar hands back the "Copy" the selection offers, and the clipboard
// records what that copy put on it.
internal class SelectionToolbar : TextToolbar {
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
internal class HeldClipboard : Clipboard {
    var entry: ClipEntry? = null
    override val nativeClipboard: Any get() = throw UnsupportedOperationException()
    override suspend fun getClipEntry(): ClipEntry? = entry
    override suspend fun setClipEntry(clipEntry: ClipEntry?) { entry = clipEntry }

    val pasted: String?
        get() = (entry?.nativeClipEntry as? Transferable)?.getTransferData(DataFlavor.stringFlavor) as String?
}

// A long press is what starts a selection on a phone, and it is a real wall-clock wait: the
// gesture is timed off the scene's own clock, which only a rendered frame advances.
@OptIn(ExperimentalComposeUiApi::class)
private fun copiedFromTheTranscript(pane: PaneState, fromY: Float = 255f): String? {
    val toolbar = SelectionToolbar()
    val clipboard = HeldClipboard()
    withScene(
        PORTRAIT.first, PORTRAIT.second, SoftTheme, TypeScale.Phone,
        content = {
            CompositionLocalProvider(
                LocalTextToolbar provides toolbar,
                LocalClipboard provides clipboard,
            ) {
                ConversationView(pane, demoInfo(), Modifier.fillMaxSize())
            }
        },
        body = { scene ->
            repeat(3) { scene.render() }
            var at = 0L
            fun touch(kind: PointerEventType, x: Float, y: Float) {
                scene.sendPointerEvent(kind, Offset(x, y), timeMillis = at, type = PointerType.Touch)
                at += 50
            }
            touch(PointerEventType.Press, 70f, fromY)
            Thread.sleep(900)
            at = 900
            scene.render()
            touch(PointerEventType.Move, 400f, 400f)
            touch(PointerEventType.Move, 740f, 760f)
            touch(PointerEventType.Release, 740f, 760f)
            scene.render()
            toolbar.copy?.invoke()
            repeat(4) { scene.render(); Thread.sleep(60) }
        },
    )
    return clipboard.pasted
}

@Composable
private fun Transcript(pane: PaneState) {
    CompositionLocalProvider(
        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
        LocalPaneIo provides RecordingIo,
    ) {
        Box(Modifier.fillMaxSize()) { ConversationView(pane, demoInfo(), Modifier.fillMaxSize()) }
    }
}

// What a reader asked for and could not do: an answer they can see is an answer they can drag
// across and copy, on the surface where the agent's words actually live.
@OptIn(ExperimentalTestApi::class)
class SelectionTest {
    @Test
    fun anAnswerInTheTranscriptCanBeDraggedAcrossAndSelected() {
        assertTrue(
            draggedAcrossTheTranscript(paneOf(ANSWER)) > 0,
            "dragging across the transcript highlighted nothing",
        )
    }

    // Selection claims the pointer, and the transcript is not only prose: a tool card opens on a
    // tap, and a card that stopped opening is a worse transcript than one that cannot be copied.
    @Test
    fun aToolCardStillOpensOnATapWhileTheTranscriptIsSelectable() = runComposeUiTest {
        setContent { Transcript(paneOf(CLAUDE_CONVO)) }
        assertTrue(
            onAllNodesWithText("herdr pane list --json", substring = true).fetchSemanticsNodes().isEmpty(),
            "the tool card was already open",
        )
        onNodeWithContentDescription("Show the output of Bash, list panes, 2 lines").performClick()
        // Scrolled to rather than asserted where it fell: expanding a card deliberately does not
        // move the transcript (AttachmentScrollTest), so a card opened near the end puts its own
        // output below the fold — which is the reader's scroll to make, not a card that failed.
        onAllNodes(hasScrollAction()).onFirst()
            .performScrollToNode(hasText("herdr pane list --json", substring = true))
        onNodeWithText("herdr pane list --json", substring = true).assertIsDisplayed()
    }

    @Test
    fun theCopyButtonOnACodeBlockStillPressesWhileTheTranscriptIsSelectable() = runComposeUiTest {
        setContent { Transcript(paneOf(CLAUDE_CONVO)) }
        onNodeWithContentDescription("Show the output of Bash, list panes, 2 lines").performClick()
        onNodeWithContentDescription("Copy the bash block").performClick()
        onNodeWithContentDescription("Copied").assertIsDisplayed()
    }

    @Test
    fun copyingASelectedAnswerYieldsTheWordsTheReaderSaw() {
        val pasted = copiedFromTheTranscript(paneOf(ANSWER))
        assertTrue(
            pasted != null && "min where max was meant" in pasted,
            "copying a selected answer produced ${pasted ?: "nothing"}",
        )
    }

    // A button's caption is chrome, not the agent's words. Dragging across a card used to splice
    // "Copy" into the middle of the code it sits above, and "Show image" into a file name — which
    // is exactly the paste a reader would then put into a bug report.
    @Test
    fun aButtonsCaptionIsNotSplicedIntoWhatTheReaderCopied() {
        // Both of these open on the same head, so the first selectable line of each is in the
        // same place; this one's is its prose, above the code card.
        val pasted = copiedFromTheTranscript(paneOf(CARDS), fromY = 255f)
        assertTrue(pasted != null, "copying across the cards produced nothing")
        assertTrue("herdr pane list --json" in pasted!!, "the code itself was not copied: $pasted")
        assertTrue("shot.png" in pasted, "the attachment's name was not copied: $pasted")
        assertTrue("Copy" !in pasted, "the copy button's caption was copied: $pasted")
        assertTrue("Show image" !in pasted, "the attachment button's caption was copied: $pasted")
    }

    @Test
    fun anAttachmentStillPressesWhileTheTranscriptIsSelectable() = runComposeUiTest {
        val frame = "{\"cursor\":\"u-1\",\"more\":false,\"pane\":\"$PANE_ID\",\"t\":\"convo\",\"turns\":[" +
            "{\"at\":\"2026-08-20T09:00:01.000Z\",\"blocks\":[{\"b\":\"md\",\"text\":\"[image · png]\"," +
            "\"att\":{\"id\":\"att-7f3\",\"kind\":\"image\",\"mime\":\"image/png\",\"bytes\":52831," +
            "\"name\":\"shot.png\"}}],\"id\":\"u-1\",\"role\":\"user\"}]}"
        val node = NodeWithAttachments(emptyMap())
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                LocalPaneIo provides node,
            ) {
                Box(Modifier.fillMaxSize()) { ConversationView(paneOf(frame), demoInfo(), Modifier.fillMaxSize()) }
            }
        }
        onNodeWithContentDescription("Show image, shot.png").performClick()
        waitUntil(timeoutMillis = 5_000) { node.asked.isNotEmpty() }
        assertEquals(listOf(PANE_ID to "att-7f3"), node.asked)
    }
}
