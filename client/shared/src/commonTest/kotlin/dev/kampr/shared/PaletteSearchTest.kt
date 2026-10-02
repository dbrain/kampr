package dev.kampr.shared

import dev.kampr.shared.model.Herd
import dev.kampr.shared.ui.PaletteItem
import dev.kampr.shared.ui.PaletteTarget
import dev.kampr.shared.ui.Screen
import dev.kampr.shared.ui.paletteItems
import dev.kampr.shared.ui.paletteSearch
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private val HERD = Herd(
    nodes = listOf(
        NodeInfo("01JLAP", "comingclean", kind = "local"),
        NodeInfo("01JHUB", "giftofthemagi2"),
        NodeInfo("01JNAS", "threnody"),
    ),
    panes = listOf(
        PaneInfo("01JLAP/w1:p1", "01JLAP", workspace = "kampr", tab = "1", cwd = "/home/dbrain/dev/kampr", agent = "claude"),
        PaneInfo("01JLAP/w2:p1", "01JLAP", workspace = "notes", tab = "1", cwd = "/home/dbrain/notes"),
        PaneInfo("01JHUB/w1:p1", "01JHUB", workspace = "herdr", tab = "1", cwd = "/home/dbrain/dev/herdr", agent = "claude"),
        PaneInfo("01JNAS/w1:p1", "01JNAS", workspace = "media", tab = "1", cwd = "/srv/jellyfin/config", cmd = "btop"),
    ),
)

private fun search(query: String, mosaic: Boolean = true, canCreate: Boolean = true): List<PaletteItem> =
    paletteSearch(paletteItems(HERD, mosaic, canCreate), query)

private fun PaletteItem.paneId() = (target as? PaletteTarget.OpenPane)?.paneId

class PaletteSearchTest {
    @Test
    fun eachKindOfNameFindsItsPaneFirst() {
        val table = listOf(
            "magi" to "01JHUB/w1:p1",
            "threnody" to "01JNAS/w1:p1",
            "notes" to "01JLAP/w2:p1",
            "jellyfin" to "01JNAS/w1:p1",
            "btop" to "01JNAS/w1:p1",
            "hrdr" to "01JHUB/w1:p1",
            "comingclean kampr" to "01JLAP/w1:p1",
            "Coming Notes" to "01JLAP/w2:p1",
        )
        for ((query, pane) in table) assertEquals(pane, search(query).first().paneId(), query)
    }

    @Test
    fun everyWordHasToLandSomewhere() {
        assertTrue(search("kampr threnody").none { it.paneId() != null }, "a pane matched only half the query")
        assertTrue(search("zzzq").isEmpty())
    }

    @Test
    fun anEmptyQueryIsTheSidebarThenThePlaces() {
        val all = search("")
        assertEquals(HERD.panes.size, all.takeWhile { it.paneId() != null }.size)
        assertEquals(PaletteTarget.Go(Screen.Fleet), all[HERD.panes.size].target)
    }

    @Test
    fun placesAreOnlyOfferedWhereTheyExist() {
        assertEquals(PaletteTarget.Go(Screen.Mosaic), search("mosaic").first().target)
        assertTrue(search("mosaic", mosaic = false).none { it.target == PaletteTarget.Go(Screen.Mosaic) })
        assertEquals(PaletteTarget.NewOn("01JNAS"), search("new threnody").first().target)
        assertTrue(search("new", canCreate = false).none { it.target is PaletteTarget.NewOn })
    }
}
