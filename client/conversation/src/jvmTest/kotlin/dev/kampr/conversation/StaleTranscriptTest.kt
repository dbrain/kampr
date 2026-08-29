package dev.kampr.conversation

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Turn
import org.jetbrains.skia.Bitmap
import java.io.File
import kotlin.math.abs
import kotlin.test.Test
import kotlin.test.assertTrue

private fun said(id: String, role: String, text: String) = Turn(id, role, null, listOf(Block.Md(text)))

// The grid draws its own ground back over itself when frames stop, and the transcript did nothing
// at all — on the one surface whose age a reader cannot judge by looking at it, because a message
// from an hour ago is drawn exactly like a message from a second ago.
//
// Measured rather than eyeballed. "Ink" is how far a pixel stands off the ground, summed over the
// render; a wash is that sum falling. Counting it rather than sampling a point keeps the test off
// any particular glyph, so re-wrapping the fixture cannot silently turn it green.
class StaleTranscriptTest {
    private fun ink(stale: Boolean, name: String): Long {
        val ground = tokensFor(SoftTheme, TypeScale.Phone, Ground.Dark).color.bg.toArgb()
        val store = KamprStore()
        store.accept(
            ServerMsg.Convo(
                pane = PANE_ID, cursor = "u-1", more = false,
                turns = listOf(
                    said("u-1", "user", "which of the two paths to herdr is dead?"),
                    said(
                        "a-2", "assistant",
                        "The socket answers and the spawned binary does not, which is why the node " +
                            "looks healthy and every pane it serves is blank.",
                    ),
                ),
            ),
        )
        if (stale) {
            store.accept(
                ServerMsg.GridReset(PANE_ID, 80, 24, emptyList(), Cursor(), emptyList()),
            )
            store.pane(PANE_ID).markStale()
        }
        val image = renderArtboard(
            390.dp, 700.dp, SoftTheme, TypeScale.Phone, File("build/artboards/transcript-$name.png"),
        ) {
            ConversationView(store.pane(PANE_ID), demoInfo(status = "idle"), Modifier.fillMaxSize())
        }
        val bitmap = Bitmap.makeFromImage(image)
        var total = 0L
        for (y in 0 until bitmap.height) {
            for (x in 0 until bitmap.width) {
                val px = bitmap.getColor(x, y)
                val off = maxOf(
                    abs(((px shr 16) and 0xFF) - ((ground shr 16) and 0xFF)),
                    abs(((px shr 8) and 0xFF) - ((ground shr 8) and 0xFF)),
                    abs((px and 0xFF) - (ground and 0xFF)),
                )
                total += off.toLong()
            }
        }
        return total
    }

    @Test
    fun aTranscriptNobodyIsStillSendingIsDrawnFadedRatherThanAsIfItWereLive() {
        val live = ink(stale = false, name = "live")
        val stale = ink(stale = true, name = "stale")
        assertTrue(live > 0, "the live transcript painted nothing off the ground, so this probe proves nothing")
        val ratio = stale.toDouble() / live.toDouble()
        assertTrue(
            ratio < 0.9,
            "a stale transcript was drawn at ${"%.2f".format(ratio)} of a live one's contrast — " +
                "the reader cannot tell it stopped arriving",
        )
        assertTrue(
            ratio > 0.2,
            "a stale transcript faded to ${"%.2f".format(ratio)} of a live one — that is not readable",
        )
    }
}
