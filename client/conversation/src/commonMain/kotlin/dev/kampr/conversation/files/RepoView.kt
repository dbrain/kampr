package dev.kampr.conversation.files

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.items
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.net.RepoSummary
import dev.kampr.shared.net.decodeRepo
import dev.kampr.shared.net.repoAttachmentId
import dev.kampr.shared.net.wallClockMillis
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.FilesNav
import dev.kampr.shared.ui.FilesPlace
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.util.relativeSeconds

@Composable
fun RepoView(place: FilesPlace.Repo, nav: FilesNav, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    var attempt by remember(place) { mutableIntStateOf(0) }
    val load = rememberLoad(place.anchor, repoAttachmentId(place.path), attempt) { got -> decodeRepo(got.bytes) }
    val root = (load as? Load.Ready)?.value?.root ?: place.path

    Column(modifier.fillMaxSize().background(tokens.color.bg)) {
        FilesBar(baseName(root), headline((load as? Load.Ready)?.value) ?: root, nav)
        LoadGate(load, "this repository", { attempt++ }) { repo -> Summary(place, repo, nav) }
    }
}

private fun headline(repo: RepoSummary?): String? {
    repo ?: return null
    val branch = repo.branch ?: "detached at ${repo.head ?: "?"}"
    val upstream = repo.upstream?.let { up ->
        val drift = listOfNotNull("${repo.ahead} ahead".takeIf { repo.ahead > 0 }, "${repo.behind} behind".takeIf { repo.behind > 0 })
        if (drift.isEmpty()) "even with $up" else "${drift.joinToString(", ")} $up"
    }
    return listOfNotNull(branch, upstream).joinToString(" · ")
}

@Composable
private fun Summary(place: FilesPlace.Repo, repo: RepoSummary, nav: FilesNav) {
    val tokens = Kampr.tokens
    val safe = LocalSafeArea.current
    val groups = remember(repo) { groupChanges(repo.changes) }
    val now = remember(repo) { wallClockMillis() }
    fun file(abs: String) = nav.open(FilesPlace.File(place.nodeId, place.anchor, abs))

    LazyColumn(
        Modifier.fillMaxSize(),
        contentPadding = PaddingValues(start = 12.dp + safe.left, end = 12.dp + safe.right, bottom = 16.dp + safe.bottom),
    ) {
        if (repo.changes.isEmpty()) {
            item(key = "clean") {
                KText("Nothing has changed since ${repo.head ?: "HEAD"}.", tokens.type.captionSmall, tokens.color.mute, Modifier.padding(vertical = 12.dp))
            }
        }
        changes("staged", "Staged", groups.staged, ::file)
        changes("unstaged", "Unstaged", groups.unstaged, ::file)
        changes("untracked", "Untracked", groups.untracked, ::file)
        if (repo.log.isNotEmpty()) item(key = "log") { SectionLabel("Recent commits") }
        items(repo.log, key = { "log-${it.sha}" }) { commit ->
            val ago = relativeSeconds(commit.`when`, now)
            EntryRow(
                FilesIcons.commit,
                commit.subject.ifEmpty { commit.short },
                "${commit.short} · ${commit.author} · $ago",
                null,
                "Commit ${commit.short}, ${commit.subject}, by ${commit.author}, $ago ago",
            ) { nav.open(FilesPlace.Commit(place.nodeId, place.anchor, repo.root, commit.sha)) }
        }
    }
}

private fun LazyListScope.changes(key: String, title: String, list: List<Changed>, onOpen: (String) -> Unit) {
    if (list.isEmpty()) return
    item(key = key) { SectionLabel("$title · ${list.size}") }
    items(list, key = { "$key-${it.change.path}" }) { changed ->
        val change = changed.change
        val renamed = change.orig?.let { "from $it" }
        EntryRow(
            null,
            change.path,
            renamed,
            changed.letter,
            "${title.lowercase()}, ${gitWord(changed.letter)}, ${change.path}${renamed?.let { ", $it" }.orEmpty()}",
        ) { onOpen(change.abs) }
    }
}
