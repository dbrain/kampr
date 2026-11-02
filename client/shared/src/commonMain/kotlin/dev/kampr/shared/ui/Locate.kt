package dev.kampr.shared.ui

import dev.kampr.shared.model.Lookup
import dev.kampr.shared.model.bestMatch
import dev.kampr.shared.model.resolvePath
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.net.AttachmentSlice
import dev.kampr.shared.net.decodeFound
import dev.kampr.shared.net.fileAttachmentId
import dev.kampr.shared.net.findAttachmentId

// Where a relative path that did not resolve against the pane's directory actually is: joined to
// each directory the reply named, nearest first, and failing that searched for below the pane's
// directory and those. Null is the same not-found the direct fetch already gave.
suspend fun locate(io: PaneIo, anchor: String, lookup: Lookup): String? {
    for (dir in lookup.near) {
        val joined = resolvePath(lookup.tail, dir)
        if (io.range(anchor, fileAttachmentId(joined), 0, 0) is AttachmentSlice.Ok) return joined
    }
    // The node refuses a tail that is not plain segments below its base.
    if (lookup.tail.split('/').any { it == "." || it == ".." || it.isEmpty() }) return null
    val here = lookup.cwd?.takeIf { it.startsWith("/") || it.startsWith("~") } ?: "~"
    val found = (listOf(here) + lookup.near).distinct().flatMap { base ->
        val got = io.attachment(anchor, findAttachmentId(base, lookup.tail)) as? AttachmentBytes.Ok
        got?.let { decodeFound(it.bytes) }?.matches.orEmpty()
    }
    return bestMatch(found)
}
