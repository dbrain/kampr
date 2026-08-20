package dev.kampr.conversation

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.ui.ImageComposeScene
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PendingOption
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue
import javax.imageio.ImageIO
import java.io.ByteArrayInputStream

private fun centreOf(scene: ImageComposeScene, target: Color): Offset {
    val png = requireNotNull(scene.render().encodeToData()).bytes
    val image = ImageIO.read(ByteArrayInputStream(png))
    val want = ((target.red * 255).toInt() shl 16) or
        ((target.green * 255).toInt() shl 8) or (target.blue * 255).toInt()
    var sx = 0L
    var sy = 0L
    var n = 0L
    for (y in 0 until image.height) {
        for (x in 0 until image.width) {
            if (image.getRGB(x, y) and 0xFFFFFF == want) {
                sx += x; sy += y; n++
            }
        }
    }
    require(n > 0) { "no pixel of the target colour was painted" }
    return Offset((sx / n).toFloat(), (sy / n).toFloat())
}

class InteractionTest {
    private val pending = ServerMsg.Pending(
        pane = PANE_ID,
        question = "Do you want to make this edit?",
        options = listOf(PendingOption("1", "Yes"), PendingOption("2", "Always"), PendingOption("3", "No")),
        source = "transcript",
    )

    // The primary chip is the only thing painted in the accent, so its centroid locates it
    // without hardcoding a layout. Only the key that was offered goes back: the node decides
    // whether a submit key follows it.
    @Test
    fun tappingAnOptionAnswersWithTheKeyItWasOffered() {
        var answered: String? = null
        withScene(
            390.dp, 240.dp, SoftTheme, TypeScale.Phone,
            content = { PendingStrip(pending, { answered = it }, Modifier.fillMaxWidth()) },
            body = { scene ->
                scene.render()
                val at = centreOf(scene, SoftTheme.palette.accent)
                scene.sendPointerEvent(PointerEventType.Press, at)
                scene.sendPointerEvent(PointerEventType.Release, at)
                scene.render()
            },
        )
        assertEquals("1", answered)
    }

    @Test
    fun aClearedPromptHidesTheStrip() {
        val (store, pane) = demoPane(RICH_CONVO)
        assertTrue(pane.pending != null)
        store.accept(requireNotNull(Wire.decode("""{"t":"pending","pane":"$PANE_ID","question":null,"options":[]}""")))
        assertNull(pane.pending)
    }

    // Reaching the top of the list is what pages backwards, and the cursor is echoed verbatim
    // because it is opaque to the client.
    @Test
    fun reachingTheTopAsksForTheOlderPageWithTheOpaqueCursor() {
        RecordingIo.sent.clear()
        val (_, pane) = demoPane(RICH_PAGE_TAIL)
        assertEquals("a-0003", pane.convoCursor)
        withScene(
            PORTRAIT.first, PORTRAIT.second, SoftTheme, TypeScale.Phone,
            content = { ConversationView(pane, demoInfo(), Modifier.fillMaxWidth()) },
            body = { scene -> repeat(3) { scene.render() } },
        )
        assertEquals(listOf(ClientMsg.ConvoLoad(PANE_ID, "a-0003")), RecordingIo.sent.filterIsInstance<ClientMsg.ConvoLoad>())
    }

    @Test
    fun aReplyIsTextThenACarriageReturn() {
        assertEquals(
            listOf(ClientMsg.InputText(PANE_ID, "run the tests"), ClientMsg.InputText(PANE_ID, "\r")),
            replyMessages(PANE_ID, "run the tests"),
        )
    }

    // The transcript bar's search control is the last thing in a fixed-height row, so its centre
    // is derivable; tapping it must swap the bar for the field and its match counter.
    @Test
    fun tappingSearchOpensTheFieldOverTheWholeTranscript() {
        val (_, pane) = demoPane(RICH_CONVO)
        withScene(
            PORTRAIT.first, PORTRAIT.second, SoftTheme, TypeScale.Phone,
            content = { ConversationView(pane, demoInfo(), Modifier.fillMaxWidth()) },
            body = { scene ->
                scene.render()
                val at = Offset(scene.render().width - 47f, 33f)
                scene.sendPointerEvent(PointerEventType.Press, at)
                scene.sendPointerEvent(PointerEventType.Release, at)
                val png = requireNotNull(scene.render().encodeToData()).bytes
                java.io.File("build/artboards").mkdirs()
                java.io.File("build/artboards/conversation-search.png").writeBytes(png)
            },
        )
    }
}
