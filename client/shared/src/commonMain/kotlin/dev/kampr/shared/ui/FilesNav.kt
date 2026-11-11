package dev.kampr.shared.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.ProvidableCompositionLocal
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import dev.kampr.shared.model.Lookup
import dev.kampr.shared.wire.Attachment

sealed interface FilesPlace {
    val nodeId: String
    val anchor: String

    data class Dir(override val nodeId: String, override val anchor: String, val path: String) : FilesPlace

    @Immutable
    data class File(override val nodeId: String, override val anchor: String, val path: String, val lookup: Lookup? = null) : FilesPlace
    data class Repo(override val nodeId: String, override val anchor: String, val path: String) : FilesPlace
    data class Commit(override val nodeId: String, override val anchor: String, val repo: String, val sha: String) : FilesPlace
    data class Record(override val nodeId: String, override val anchor: String, val att: Attachment) : FilesPlace
}

interface FilesNav {
    fun open(place: FilesPlace)
    fun replace(place: FilesPlace)
    fun back()
    fun close()
}

val LocalOpenPlace: ProvidableCompositionLocal<(FilesPlace) -> Unit> = staticCompositionLocalOf { {} }

fun nodeOfPane(paneId: String): String = paneId.substringBefore('/')

fun filePlace(paneId: String, path: String, lookup: Lookup? = null): FilesPlace.File = FilesPlace.File(nodeOfPane(paneId), paneId, path, lookup)

fun recordPlace(paneId: String, att: Attachment): FilesPlace.Record = FilesPlace.Record(nodeOfPane(paneId), paneId, att)

internal class AppFilesNav(private val state: AppState) : FilesNav {
    override fun open(place: FilesPlace) = state.open(place)
    override fun replace(place: FilesPlace) = state.replaceFile(place)
    override fun back() = state.backFiles()
    override fun close() = state.closeFiles()
}

@Composable
internal fun FilesLayer(state: AppState, surfaces: PaneSurfaces, nav: FilesNav) {
    FilesStack(state.files) { surfaces.Files(it, nav, Modifier.modal(nav::back)) }
}

// Keyed by depth as well as place: the same folder twice on one stack is two visits, each with
// its own filter and scroll.
@Composable
fun FilesStack(places: List<FilesPlace>, content: @Composable (FilesPlace) -> Unit) {
    val keys = places.mapIndexed { depth, place -> "$depth $place" }
    val kept = rememberKept(keys)
    val top = places.lastOrNull() ?: return
    kept.Keep(keys.last()) { content(top) }
}
