package dev.kampr.shared.ui

import dev.kampr.shared.model.AgentStatus
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.groups
import dev.kampr.shared.model.homeRelative
import dev.kampr.shared.model.paneTitle
import dev.kampr.shared.model.statusOf
import dev.kampr.shared.wire.PaneInfo

sealed interface PaletteTarget {
    data class OpenPane(val paneId: String) : PaletteTarget
    data class Go(val screen: Screen) : PaletteTarget
    data class NewOn(val nodeId: String) : PaletteTarget
}

data class PaletteField(val text: String, val weight: Int)

data class PaletteItem(
    val title: String,
    val detail: String,
    val target: PaletteTarget,
    val fields: List<PaletteField>,
    val status: AgentStatus? = null,
)

// Nine, because a result is picked by its digit and there is no tenth digit to press.
const val PALETTE_SHOWN = 9

fun paletteItems(herd: Herd, mosaic: Boolean, canCreate: Boolean): List<PaletteItem> {
    val names = herd.nodes.associate { it.id to it.name }
    val listed = herd.groups().flatMap { it.panes }
    val fleet = herd.panes.filter { it.fleet != null }
    val panes = (listed + fleet).map { paneItem(it, names[it.nodeId] ?: it.nodeId) }
    val places = buildList {
        add(place("Fleet", "one command across the herd", Screen.Fleet, "fleet run broadcast"))
        if (mosaic) add(place("Mosaic", "several panes at once", Screen.Mosaic, "grid tile split"))
        add(place("Settings", "address, pairing, machines", Screen.Setup, "setup nodes machines pair"))
        add(place("Devices", "paired devices", Screen.Devices, "tokens revoke"))
        add(place("Appearance", "theme and mode", Screen.Appearance, "theme dark light colour"))
        add(place("Notifications", "push and alerts", Screen.Notifications, "push alerts"))
        if (canCreate) {
            for (node in herd.nodes.filter { it.online }) {
                add(
                    PaletteItem(
                        "New on ${node.name}",
                        "workspace, tab or agent",
                        PaletteTarget.NewOn(node.id),
                        listOf(PaletteField("new create workspace session", 2), PaletteField(node.name, 3)),
                    ),
                )
            }
        }
    }
    return panes + places
}

private fun paneItem(pane: PaneInfo, node: String): PaletteItem {
    val title = paneTitle(pane)
    val path = pane.cwd?.let(::homeRelative).orEmpty()
    val where = listOfNotNull(node, pane.workspace, pane.tab?.let { "tab $it" }).joinToString(" · ")
    return PaletteItem(
        title = title,
        detail = listOf(where, path).filter { it.isNotEmpty() }.joinToString("  "),
        target = PaletteTarget.OpenPane(pane.id),
        status = statusOf(pane),
        fields = listOfNotNull(
            PaletteField(title, 3),
            PaletteField(node, 3),
            pane.workspace?.let { PaletteField(it, 3) },
            pane.label?.let { PaletteField(it, 3) },
            pane.title?.let { PaletteField(it, 3) },
            pane.tab?.let { PaletteField(it, 2) },
            pane.agent?.let { PaletteField(it, 2) },
            pane.cwd?.substringAfterLast('/')?.let { PaletteField(it, 2) },
            PaletteField(path, 1),
            pane.cmd?.let { PaletteField(it, 1) },
            pane.argv?.let { PaletteField(it, 1) },
            pane.fleet?.let { PaletteField("fleet ${it.command}", 1) },
        ),
    )
}

private fun place(title: String, detail: String, screen: Screen, words: String) = PaletteItem(
    title, detail, PaletteTarget.Go(screen),
    listOf(PaletteField(title, 3), PaletteField(words, 1)),
)

// Every word typed has to land somewhere in the item, and an item ranks by how well each landed.
// Ties keep the input's order, which for an empty query is the sidebar's.
fun paletteSearch(items: List<PaletteItem>, query: String): List<PaletteItem> {
    val words = query.lowercase().split(' ').filter { it.isNotEmpty() }
    if (words.isEmpty()) return items
    return items
        .map { item -> item to words.sumOf { word -> bestScore(item, word) ?: return@map item to -1 } }
        .filter { it.second > 0 }
        .sortedByDescending { it.second }
        .map { it.first }
}

private fun bestScore(item: PaletteItem, word: String): Int? =
    item.fields.maxOfOrNull { field -> fuzzy(field.text.lowercase(), word) * field.weight }?.takeIf { it > 0 }

internal fun fuzzy(text: String, word: String): Int {
    if (word.isEmpty() || text.isEmpty()) return 0
    if (text.startsWith(word)) return 100
    var at = text.indexOf(word)
    while (at > 0) {
        if (!text[at - 1].isLetterOrDigit()) return 80
        at = text.indexOf(word, at + 1)
    }
    if (text.contains(word)) return 60
    var cursor = 0
    var first = -1
    for (ch in word) {
        val found = text.indexOf(ch, cursor)
        if (found < 0) return 0
        if (first < 0) first = found
        cursor = found + 1
    }
    return 10 + 30 * word.length / (cursor - first)
}
