package dev.kampr.terminal

import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.theme.AllThemes
import dev.kampr.shared.theme.TerminalPalette
import dev.kampr.shared.theme.terminalSkin
import dev.kampr.shared.wire.ColorSpec
import dev.kampr.shared.wire.Style
import dev.kampr.terminal.render.ResolvedStyles
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotEquals

private fun pen(slot: Int) = Style(bg = ColorSpec.Indexed(slot))

private fun resolved() = ResolvedStyles(TerminalPalette(terminalSkin(AllThemes.first().id)))

// The renderer reads a colour per cell per frame and cannot afford to resolve one, so the table is
// resolved once and re-resolved when it moves. What "it moved" meant was that it had grown — true
// while a connection lasts, and false across two of them: the node interns per socket, so a
// reconnect that meets fewer pens than the last one hands back a table of exactly the same size
// with different colours in it, and the grid went on being painted in the pens of a socket that
// had already closed.
class StyleSyncTest {
    @Test
    fun aTableThatChangedWithoutGrowingIsResolvedAgain() {
        val table = StyleTable()
        table.append(1, listOf(pen(1), pen(2)))
        val styles = resolved()
        styles.sync(table)
        val before = styles.bg[2]

        table.append(1, listOf(pen(4), pen(5)))
        styles.sync(table)
        assertNotEquals(before, styles.bg[2], "the pen the socket before interned is still on screen")
    }

    @Test
    fun aTableThatWasResetHandsBackTheDefaultGroundForEveryIdItNoLongerHolds() {
        val table = StyleTable()
        table.append(1, listOf(pen(1), pen(2), pen(3)))
        val styles = resolved()
        styles.sync(table)

        table.reset()
        styles.sync(table)
        assertEquals(styles.defaultBg, styles.bg[styles.clamp(3)], "a stale id must paint nothing")
    }
}
