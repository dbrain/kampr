package dev.kampr.conversation.files

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.conversation.ConversationIcons
import dev.kampr.shared.net.FsListing
import dev.kampr.shared.net.RepoHead
import dev.kampr.shared.net.UploadResult
import dev.kampr.shared.net.UploadSource
import dev.kampr.shared.net.decodeListing
import dev.kampr.shared.net.lsAttachmentId
import dev.kampr.shared.net.wallClockMillis
import dev.kampr.shared.platform.LocalReduceMotion
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.FilesNav
import dev.kampr.shared.ui.FilesPlace
import dev.kampr.shared.ui.GlyphTarget
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.KField
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.KamprIcons
import dev.kampr.shared.ui.LANDSCAPE_TOUCH
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.announce
import dev.kampr.shared.ui.touchable
import dev.kampr.shared.util.relativeSeconds
import kotlinx.coroutines.launch

// Below this many entries a filter is one more thing between the thumb and the list.
private const val FILTER_FROM = 12

@Composable
fun DirView(place: FilesPlace.Dir, nav: FilesNav, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val io = LocalPaneIo.current
    val picker = LocalUploadPicker.current
    val scope = rememberCoroutineScope()
    var attempt by remember(place) { mutableIntStateOf(0) }
    var showHidden by remember { mutableStateOf(false) }
    var transfer by remember(place) { mutableStateOf<Transfer?>(null) }
    var fresh by remember(place) { mutableStateOf<String?>(null) }
    val load = rememberLoad(place.anchor, lsAttachmentId(place.path), attempt) { got -> decodeListing(got.bytes) }
    val shown = (load as? Load.Ready)?.value?.path ?: place.path
    val name = baseName(shown).ifEmpty { "/" }
    fun run(start: String, work: suspend () -> Transfer) {
        if (transfer is Transfer.Moving) return
        transfer = Transfer.Moving(start)
        scope.launch { transfer = work() }
    }
    fun upload(pick: suspend () -> UploadSource?) {
        if (transfer is Transfer.Moving) return
        scope.launch {
            val source = pick() ?: return@launch
            transfer = Transfer.Moving(uploadProgress(source.name, 0, source.size))
            val result = io.upload(place.anchor, shown, source.name, source) { sent ->
                transfer = Transfer.Moving(uploadProgress(source.name, sent, source.size))
            }
            transfer = uploadOutcome(result)
            if (result is UploadResult.Ok) {
                fresh = baseName(result.path)
                attempt++
            }
        }
    }

    Column(modifier.fillMaxSize().background(tokens.color.bg)) {
        FilesBar(name, shown, nav) {
            GlyphTarget(
                FilesIcons.eye,
                if (showHidden) "Hide dotfiles" else "Show dotfiles",
                if (showHidden) tokens.color.accent else tokens.color.mute,
                { showHidden = !showHidden },
                target = LANDSCAPE_TOUCH,
                glyph = 15.dp,
            )
            if (!io.readOnly) {
                picker?.let { pick ->
                    GlyphTarget(TransferIcons.upload, "Upload a file into $name", tokens.color.mute, { upload(pick) }, target = LANDSCAPE_TOUCH, glyph = 15.dp)
                }
                GlyphTarget(
                    TransferIcons.zip,
                    "Download $name as a zip",
                    tokens.color.mute,
                    { run("Zipping ${zipName(shown)}…") { saveZip(io, place.anchor, shown) } },
                    target = LANDSCAPE_TOUCH,
                    glyph = 15.dp,
                )
            }
        }
        LoadGate(load, "this folder", { attempt++ }) { listing ->
            Listing(place, listing, showHidden, nav, transfer, fresh) { path ->
                run("Saving ${baseName(path)}…") { saveFile(io, place.anchor, path) }
            }
        }
    }
}

@Composable
private fun Listing(
    place: FilesPlace.Dir,
    listing: FsListing,
    showHidden: Boolean,
    nav: FilesNav,
    transfer: Transfer?,
    fresh: String?,
    onSave: (String) -> Unit,
) {
    val tokens = Kampr.tokens
    val safe = LocalSafeArea.current
    val list = rememberLazyListState()
    val still = LocalReduceMotion.current
    var query by remember(listing.path) { mutableStateOf("") }
    val entries = remember(listing, query, showHidden) { visibleEntries(listing.entries, query, showHidden) }
    val now = remember(listing) { wallClockMillis() }
    val filtered = listing.entries.size >= FILTER_FROM
    fun dir(path: String) = nav.open(FilesPlace.Dir(place.nodeId, place.anchor, path))

    val leading = listOf(true, listing.repo != null, filtered, transfer != null, listing.parent != null).count { it }
    LaunchedEffect(fresh, entries) {
        val at = entries.indexOfFirst { it.name == fresh }
        when {
            at < 0 -> Unit
            still -> list.scrollToItem(leading + at)
            else -> list.animateScrollToItem(leading + at)
        }
    }

    LazyColumn(
        Modifier.fillMaxSize(),
        state = list,
        contentPadding = PaddingValues(start = 12.dp + safe.left, end = 12.dp + safe.right, bottom = 16.dp + safe.bottom),
    ) {
        item(key = "crumbs") { Crumbs(crumbs(listing.path, listing.home), ::dir) }
        listing.repo?.let { repo ->
            item(key = "repo") { RepoStrip(repo) { nav.open(FilesPlace.Repo(place.nodeId, place.anchor, repo.root)) } }
        }
        if (filtered) {
            item(key = "filter") {
                KField("filter", query, Modifier.padding(vertical = 6.dp), label = "Filter this folder") { query = it }
            }
        }
        transfer?.let { note ->
            item(key = "transfer") {
                val tone = when (note) {
                    is Transfer.Moving -> tokens.color.working
                    is Transfer.Done -> tokens.color.done
                    is Transfer.Failed -> tokens.color.blocked
                }
                KText(note.words, tokens.type.captionSmall, tone, Modifier.padding(vertical = 6.dp).announce(note.words), maxLines = 3)
            }
        }
        listing.parent?.let { parent ->
            item(key = "up") {
                EntryRow(FilesIcons.up, "..", null, null, "Up to ${baseName(parent).ifEmpty { "/" }}") { dir(parent) }
            }
        }
        items(entries, key = { it.name }) { entry ->
            val path = childPath(listing.path, entry.name)
            val facts = listOfNotNull(
                entry.bytes?.takeIf { !entry.isDir }?.let(::formatBytes),
                entry.mtime?.let { relativeSeconds(it, now) },
            ).joinToString(" · ")
            val git = entry.git
            val isFresh = entry.name == fresh
            val spoken = listOfNotNull(
                if (entry.isDir) "folder ${entry.name}" else entry.name,
                git?.let { if (entry.isDir) "has changes" else gitWord(it) },
                facts.takeIf { it.isNotEmpty() },
                "just uploaded".takeIf { isFresh },
            ).joinToString(", ")
            val mark = if (isFresh) Modifier.background(tokens.color.accentSoft, RoundedCornerShape(tokens.radii.sm)) else Modifier
            if (entry.isDir) {
                EntryRow(KamprIcons.folder, entry.name, facts, git?.let { "•" }, spoken, modifier = mark) { dir(path) }
            } else {
                val fetch = { onSave(path) }
                EntryRow(
                    FilesIcons.file,
                    entry.name,
                    facts,
                    git,
                    spoken,
                    onLongClick = fetch,
                    trailing = {
                        GlyphTarget(
                            ConversationIcons.download,
                            "Save ${entry.name} to this device",
                            tokens.color.mute,
                            fetch,
                            target = LANDSCAPE_TOUCH,
                            glyph = 14.dp,
                        )
                    },
                    modifier = mark,
                ) { nav.open(FilesPlace.File(place.nodeId, place.anchor, path)) }
            }
        }
        if (entries.isEmpty()) {
            item(key = "empty") {
                val words = if (listing.entries.isEmpty()) "This folder is empty." else "Nothing here matches, or only dotfiles do."
                KText(words, tokens.type.captionSmall, tokens.color.mute, Modifier.padding(vertical = 12.dp), maxLines = 2)
            }
        }
        if (listing.truncated) {
            item(key = "truncated") {
                KText(
                    "Showing the first ${listing.entries.size} entries; the folder holds more.",
                    tokens.type.captionSmall,
                    tokens.color.mute,
                    Modifier.padding(vertical = 12.dp),
                    maxLines = 2,
                )
            }
        }
    }
}

@Composable
private fun Crumbs(trail: List<Crumb>, onOpen: (String) -> Unit) {
    val tokens = Kampr.tokens
    Row(
        Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        trail.forEachIndexed { index, crumb ->
            if (index > 0 && trail[index - 1].label != "/") KText("/", tokens.type.meta, tokens.color.mute)
            val last = index == trail.lastIndex
            KText(
                crumb.label,
                tokens.type.meta,
                if (last) tokens.color.text else tokens.color.accent,
                Modifier
                    .touchable(LANDSCAPE_TOUCH)
                    .action(if (crumb.label == "~") "Home folder" else "Folder ${crumb.path}", { onOpen(crumb.path) })
                    .padding(horizontal = 6.dp, vertical = 9.dp),
            )
        }
    }
}

@Composable
private fun RepoStrip(repo: RepoHead, onOpen: () -> Unit) {
    val tokens = Kampr.tokens
    val branch = repo.branch ?: "detached at ${repo.head ?: "?"}"
    val drift = listOfNotNull(
        "↑${repo.ahead}".takeIf { repo.ahead > 0 },
        "↓${repo.behind}".takeIf { repo.behind > 0 },
    ).joinToString(" ")
    val changed = if (repo.changed == 0) "clean" else "${repo.changed} changed"
    val spoken = listOfNotNull(
        "Git branch $branch",
        "${repo.ahead} ahead".takeIf { repo.ahead > 0 },
        "${repo.behind} behind".takeIf { repo.behind > 0 },
        changed,
        "open changes and history",
    ).joinToString(", ")
    Row(
        Modifier
            .fillMaxWidth()
            .padding(vertical = 4.dp)
            .background(tokens.color.surface, RoundedCornerShape(tokens.radii.sm))
            .touchable(LANDSCAPE_TOUCH)
            .action(spoken, onOpen)
            .padding(horizontal = 11.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        IconGlyph(KamprIcons.branch, 14.dp, tokens.color.dim)
        KText(branch, tokens.type.meta, tokens.color.text, Modifier.weight(1f, fill = false))
        if (drift.isNotEmpty()) KText(drift, tokens.type.micro, tokens.color.working)
        KText(changed, tokens.type.micro, if (repo.changed == 0) tokens.color.mute else tokens.color.accent)
    }
}
