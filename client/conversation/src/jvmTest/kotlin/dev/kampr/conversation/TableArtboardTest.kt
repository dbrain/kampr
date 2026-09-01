package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.conversation.md.MdBlock
import dev.kampr.conversation.md.parseMarkdown
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.ThemeSpec
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.Wire
import java.io.File
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private val OUT = File("build/artboards")

private fun probeLogMarkdown(): String {
    val convo = Wire.decode(RICH_CONVO) as dev.kampr.shared.wire.ServerMsg.Convo
    return convo.turns[1].blocks.filterIsInstance<Block.Md>().first().text
}

class TableArtboardTest {
    private fun renderTable(width: Dp, name: String, spec: ThemeSpec = SoftTheme): Int =
        renderNarrowTable(probeLogMarkdown(), width, name, spec)

    private fun renderNarrowTable(
        markdown: String,
        width: Dp,
        name: String,
        spec: ThemeSpec = SoftTheme,
    ): Int {
        var reported = 0
        val blocks = parseMarkdown(markdown)
        renderArtboard(width, 620.dp, spec, TypeScale.Phone, File(OUT, name)) {
            val tokens = Kampr.tokens
            Column(
                Modifier
                    .fillMaxSize()
                    .background(tokens.color.bg)
                    .verticalScroll(rememberScrollState())
                    .padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                for (block in blocks) {
                    if (block is MdBlock.Table) {
                        dev.kampr.conversation.md.MarkdownTable(
                            block,
                            "",
                            Modifier.onSizeChanged { reported = it.width },
                        )
                    }
                }
            }
        }
        return reported
    }

    // The whole promise of the screen: the four-column probe-log table renders as a table on a
    // 390 px phone, and its container is exactly the page width — the overflow lives in the
    // table's own scroller, so nothing above it can move sideways.
    @Test
    fun theProbeLogTableIsContainedAt390px() {
        val reported = renderTable(390.dp, "table-390.png")
        val page = (390 - 32) * 2
        assertEquals(page, reported)
        assertTrue(reported > 0)
    }

    // The same table handed more room than it needs. It stops at its own columns rather than
    // taking whatever it is offered, and it is the same width again on a pane nearly twice as
    // wide — which is the whole of what "fit the content" means here.
    @Test
    fun theSameTableStopsGrowingOnceThePaneHasRoomForIt() {
        val page = (844 - 32) * 2
        val at844 = renderTable(844.dp, "table-844.png")
        val at1440 = renderTable(1440.dp, "table-1440.png")
        assertTrue(at844 in 1..<page, "the table grew to fill a pane it did not need: $at844 of $page")
        assertEquals(at844, at1440, "the table changed width with the pane it was drawn in")
    }

    // The report: *"things like tables should fit the content in a reasonable way, the same way
    // Claude Code itself doesn't stretch over the entire space"*.
    //
    // A narrow table used to have its columns scaled up until they filled the pane, which on a
    // desktop drew two words and a status inside a metre of ruled box. The columns are natural
    // widths now and the table is their sum, so a small table is a small table at any pane width.
    //
    // The mutation that must fail: scale the columns to `available` again and this reports the
    // page width.
    @Test
    fun aNarrowTableIsAsWideAsItsColumnsAndNoWider() {
        val markdown = "| crate | result |\n| --- | --- |\n| core | ok |\n| term | ok |\n"
        val wide = renderNarrowTable(markdown, 844.dp, "table-narrow-844.png")
        val narrow = renderNarrowTable(markdown, 390.dp, "table-narrow-390.png")
        assertTrue(wide > 0, "the table was never laid out")
        assertEquals(narrow, wide, "the table changed width with the pane it was drawn in")
        assertTrue(
            wide < (844 - 32) * 2 / 2,
            "a four-word table still fills a desktop pane: $wide",
        )
    }

    @Test
    fun theTableRendersInASecondTheme() {
        renderTable(390.dp, "table-390-phosphor.png", dev.kampr.shared.theme.PhosphorTheme)
    }
}
