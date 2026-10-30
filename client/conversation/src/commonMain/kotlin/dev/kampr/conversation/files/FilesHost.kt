package dev.kampr.conversation.files

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import dev.kampr.shared.ui.FilesNav
import dev.kampr.shared.ui.FilesPlace

@Composable
fun FilesHost(place: FilesPlace, nav: FilesNav, modifier: Modifier = Modifier) {
    when (place) {
        is FilesPlace.Dir -> DirView(place, nav, modifier)
        is FilesPlace.Repo -> RepoView(place, nav, modifier)
        is FilesPlace.Commit -> CommitView(place, nav, modifier)
        is FilesPlace.File -> FileView(place, nav, modifier)
        is FilesPlace.Record -> RecordView(place, nav, modifier)
    }
}
