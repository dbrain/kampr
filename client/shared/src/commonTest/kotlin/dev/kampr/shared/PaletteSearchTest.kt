package dev.kampr.shared

import dev.kampr.shared.model.Herd
import dev.kampr.shared.net.anchorPane
import dev.kampr.shared.ui.FilesPlace
import dev.kampr.shared.ui.PaletteItem
import dev.kampr.shared.ui.PaletteTarget
import dev.kampr.shared.ui.Screen
import dev.kampr.shared.ui.paletteItems
import dev.kampr.shared.ui.paletteSearch
import dev.kampr.shared.wire.ContainerInfo
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private val HERD = Herd(
    nodes = listOf(
        NodeInfo("01JLAP", "comingclean", kind = "local"),
        NodeInfo("01JHUB", "giftofthemagi2"),
        NodeInfo(
            "01JNAS",
            "threnody",
            containers = listOf(
                ContainerInfo(id = "f49c60fac115", name = "kobbler-postgres-test-1", image = "postgres:18.6", state = "running", project = "kobbler"),
                ContainerInfo(id = "c2", name = "old-job", image = "busybox", state = "exited"),
            ),
        ),
        NodeInfo("01JOFF", "attic", reachable = false),
    ),
    panes = listOf(
        PaneInfo("01JLAP/w1:p1", "01JLAP", workspace = "kampr", tab = "1", cwd = "/home/dbrain/dev/kampr", agent = "claude"),
        PaneInfo("01JLAP/w2:p1", "01JLAP", workspace = "notes", tab = "1", cwd = "/home/dbrain/notes"),
        PaneInfo("01JHUB/w1:p1", "01JHUB", workspace = "herdr", tab = "1", cwd = "/home/dbrain/dev/herdr", agent = "claude"),
        PaneInfo("01JNAS/w1:p1", "01JNAS", workspace = "media", tab = "1", cwd = "/srv/jellyfin/config", cmd = "btop"),
    ),
)

private fun search(query: String, mosaic: Boolean = true, canCreate: Boolean = true, canBrowse: Boolean = true): List<PaletteItem> = paletteSearch(paletteItems(HERD, mosaic, canCreate, canBrowse), query)

private fun PaletteItem.paneId() = (target as? PaletteTarget.OpenPane)?.paneId

class PaletteSearchTest {
    @Test
    fun aContainerIsFoundByItsNameImageOrProjectAndOpensItsSheet() {
        val sheet = PaletteTarget.Container("01JNAS", "f49c60fac115")
        for (query in listOf("postgres", "kobbler", "postgres:18", "threnody pg")) {
            assertEquals(sheet, search(query).first().target, query)
        }
        assertEquals(PaletteTarget.Container("01JNAS", "c2"), search("old-job").first().target, "a stopped one is still found")
    }

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
    fun anEmptyQueryIsTheSidebarThenTheContainersThenThePlaces() {
        val all = search("")
        assertEquals(HERD.panes.size, all.takeWhile { it.paneId() != null }.size)
        val containers = HERD.nodes.sumOf { it.containers.size }
        assertTrue(all.drop(HERD.panes.size).take(containers).all { it.target is PaletteTarget.Container })
        assertEquals(PaletteTarget.Go(Screen.Fleet), all[HERD.panes.size + containers].target)
    }

    @Test
    fun placesAreOnlyOfferedWhereTheyExist() {
        assertEquals(PaletteTarget.Go(Screen.Mosaic), search("mosaic").first().target)
        assertTrue(search("mosaic", mosaic = false).none { it.target == PaletteTarget.Go(Screen.Mosaic) })
        assertEquals(PaletteTarget.NewOn("01JNAS"), search("new threnody").first().target)
        assertTrue(search("new", canCreate = false).none { it.target is PaletteTarget.NewOn })
    }

    // Browsing needs the node process and nothing else: herdr being down leaves the disk there.
    @Test
    fun everyReachableNodeOffersItsFilesFromHome() {
        assertEquals(PaletteTarget.Files(FilesPlace.Dir("01JNAS", anchorPane("01JNAS"), "~")), search("files threnody").first().target)
        val offered = search("files").mapNotNull { (it.target as? PaletteTarget.Files)?.place?.nodeId }
        assertEquals(listOf("01JLAP", "01JHUB", "01JNAS"), offered)
        assertTrue(search("files", canBrowse = false).none { it.target is PaletteTarget.Files })
    }
}
