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
    private fun renderTable(width: Dp, name: String, spec: ThemeSpec = SoftTheme): Int {
        var reported = 0
        val blocks = parseMarkdown(probeLogMarkdown())
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

    @Test
    fun theSameTableFillsAWiderPane() {
        assertEquals((844 - 32) * 2, renderTable(844.dp, "table-844.png"))
    }

    @Test
    fun theTableRendersInASecondTheme() {
        renderTable(390.dp, "table-390-phosphor.png", dev.kampr.shared.theme.PhosphorTheme)
    }
}
