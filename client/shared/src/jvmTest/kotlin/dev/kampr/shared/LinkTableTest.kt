package dev.kampr.shared

import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

// Every frame in link-reset.ndjson came off a live kampr node on a herdr pane that printed three
// OSC-8 links with a `pane split` between each — the split is what makes herdr repaint and the
// node re-send the whole table from index 0. Hand-built messages agreed with the client's own
// reading of the protocol, which is exactly why this went unnoticed.
private fun frames(): List<String> =
    checkNotNull(LinkTableTest::class.java.getResourceAsStream("/link-reset.ndjson"))
        .bufferedReader().readLines().filter { it.isNotBlank() }

class LinkTableTest {
    @Test
    fun aResetCarriesTheWholeTableAndAPatchOnlyTheSuffix() {
        val store = KamprStore()
        var last: String? = null
        var resets = 0
        for (line in frames()) {
            val msg = assertNotNull(Wire.decode(line), "undecodable: ${line.take(90)}")
            store.accept(msg)
            if (msg is ServerMsg.GridReset) {
                resets++
                last = msg.pane
                assertEquals(msg.links, store.pane(msg.pane).links, "reset $resets did not replace the table")
            }
        }
        assertTrue(resets >= 3, "the capture has to contain the repeated reset that exposed this")
        val pane = store.pane(assertNotNull(last))
        assertEquals(
            listOf("https://example.com/AAA", "https://example.com/BBB", "https://example.com/CCC"),
            pane.links,
        )
    }

    // The table is only ever read by id off a cell, so the table being wrong means a tap opens
    // whatever some earlier program printed.
    @Test
    fun theLinkOnTheLastRowPrintedResolvesToTheUrlThatRowDeclared() {
        val store = KamprStore()
        var last: String? = null
        for (line in frames()) {
            val msg = assertNotNull(Wire.decode(line))
            store.accept(msg)
            if (msg is ServerMsg.GridReset) last = msg.pane
            if (msg is ServerMsg.GridPatch) last = msg.pane
        }
        val pane = store.pane(assertNotNull(last))
        for ((label, url) in listOf("CCC-LINK" to "https://example.com/CCC")) {
            val cell = (0 until pane.cells.rows).firstNotNullOfOrNull { row ->
                (0 until pane.cells.cols).firstOrNull { col ->
                    pane.cells.linkAt(col, row) >= 0 && rowText(pane, row).contains(label)
                }?.let { it to row }
            }
            assertNotNull(cell, "$label is not on the grid")
            val id = pane.cells.linkAt(cell.first, cell.second)
            assertEquals(url, pane.links.getOrNull(id), "$label opens the wrong URL")
        }
    }
}

private fun rowText(pane: dev.kampr.shared.model.PaneState, row: Int): String =
    (0 until pane.cells.cols).map { pane.cells.charAt(it, row) }.joinToString("")
