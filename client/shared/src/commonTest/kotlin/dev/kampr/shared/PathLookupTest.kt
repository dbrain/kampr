package dev.kampr.shared

import dev.kampr.shared.model.Lookup
import dev.kampr.shared.model.bestMatch
import dev.kampr.shared.model.lookupFor
import dev.kampr.shared.model.mentionedDirs
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.net.fileAttachmentId
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.locate
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs
import kotlinx.coroutines.test.runTest
import kotlin.io.encoding.Base64
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

private const val ANCHOR = "01JNODE/w1:p1"
private const val ROOT = "/home/u/dev/kampr"

// A node holding exactly these files, answering a `file` id for each and a `find` id the way the
// node's own search does: whole trailing segments below the base, none at all being a refusal.
private class Disk(vararg files: String) : PaneIo {
    private val files = files.toList()
    val asked = mutableListOf<String>()
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String): PanePrefs = PanePrefs()
    override suspend fun attachment(paneId: String, id: String): AttachmentBytes {
        asked += id
        val fields = Base64.UrlSafe.decode(id + "=".repeat((4 - id.length % 4) % 4)).decodeToString().split('\u001F')
        val found = when {
            fields.size == 2 && fields[0] == "file" -> files.filter { it == fields[1] }.map { "x" }
            fields.size == 3 && fields[0] == "find" -> files.filter { it.startsWith("${fields[1]}/") && "/$it".endsWith("/${fields[2]}") }
            else -> emptyList()
        }
        if (found.isEmpty()) return AttachmentBytes.Failed("no such attachment")
        if (fields[0] == "file") return AttachmentBytes.Ok("x".encodeToByteArray(), null)
        val json = found.joinToString(",", """{"matches":[""", """],"truncated":false}""") { "\"$it\"" }
        return AttachmentBytes.Ok(json.encodeToByteArray(), "application/json")
    }
}

class PathLookupTest {
    // The first report: from the repository root an agent wrote the path as the crate saw it.
    @Test
    fun aPathWrittenFromADirectoryBelowTheCwdIsFoundUnderIt() = runTest {
        val disk = Disk("$ROOT/crates/kampr-node/src/fs.rs")
        val lookup = lookupFor("kampr-node/src/fs.rs", ROOT)!!
        assertEquals("$ROOT/crates/kampr-node/src/fs.rs", locate(disk, ANCHOR, lookup))
    }

    // The second report: "look in <dir>", then bare names. The directory the reply named is tried
    // before anything is searched, even where a search would have found a different file.
    @Test
    fun aBareNameAfterANamedDirectoryOpensInThatDirectory() = runTest {
        val src = "$ROOT/crates/kampr-node/src"
        val disk = Disk("$src/parser.rs", "$ROOT/aaa/parser.rs")
        assertEquals("$src/parser.rs", locate(disk, ANCHOR, lookupFor("parser.rs", ROOT, listOf(src))!!))
        assertEquals(listOf(fileAttachmentId("$src/parser.rs")), disk.asked, "searched before trying the named directory")
    }

    @Test
    fun theNearestNamedDirectoryIsTriedFirst() = runTest {
        val disk = Disk("/a/x.rs", "/b/x.rs")
        assertEquals("/b/x.rs", locate(disk, ANCHOR, lookupFor("x.rs", ROOT, listOf("/b", "/a"))!!))
    }

    @Test
    fun aNamedDirectoryIsSearchedBelowAsWellAsJoined() = runTest {
        val disk = Disk("/srv/app/src/deep/lib.rs")
        assertEquals("/srv/app/src/deep/lib.rs", locate(disk, ANCHOR, lookupFor("deep/lib.rs", ROOT, listOf("/srv/app"))!!))
    }

    @Test
    fun ofSeveralMatchesTheShallowestThenTheFirstByNameOpens() {
        assertEquals("/r/b/x.rs", bestMatch(listOf("/r/c/d/x.rs", "/r/b/x.rs", "/r/z/x.rs")))
        assertEquals("/r/a/x.rs", bestMatch(listOf("/r/b/x.rs", "/r/a/x.rs")))
        assertNull(bestMatch(emptyList()))
    }

    @Test
    fun severalMatchesUnderTheCwdOpenTheBestOne() = runTest {
        val disk = Disk("$ROOT/z/x.rs", "$ROOT/a/x.rs", "$ROOT/a/b/x.rs")
        assertEquals("$ROOT/a/x.rs", locate(disk, ANCHOR, lookupFor("x.rs", ROOT)!!))
    }

    @Test
    fun nothingAnywhereIsNothing() = runTest {
        assertNull(locate(Disk(), ANCHOR, lookupFor("x.rs", ROOT, listOf("/a"))!!))
    }

    // An absolute or home-anchored path says where it is; searching for it would open a different
    // file than the one written.
    @Test
    fun anAnchoredPathIsNeverLookedFor() {
        assertNull(lookupFor("/etc/hosts", ROOT, listOf("/a")))
        assertNull(lookupFor("~/notes.md", ROOT))
        assertEquals(Lookup("src/x.rs", ROOT, emptyList()), lookupFor("./src/x.rs", ROOT))
    }

    // A tail that climbs out of where it was written is joined to the named directories, and never
    // handed to a search, which the node would refuse.
    @Test
    fun aClimbingPathIsJoinedButNeverSearched() = runTest {
        val disk = Disk("/a/x.rs")
        assertEquals("/a/x.rs", locate(disk, ANCHOR, lookupFor("../x.rs", ROOT, listOf("/a/b"))!!))
        assertNull(locate(Disk(), ANCHOR, lookupFor("../x.rs", ROOT)!!))
    }

    @Test
    fun theDirectoriesAReplyNamesAreTheAnchoredOnesThatAreNotFiles() {
        val text = "Look in /some/dir/ (and `~/proj`), not /etc/hosts.conf or src/x.rs; also \"/srv/app\"."
        assertEquals(listOf("/some/dir", "~/proj", "/srv/app"), mentionedDirs(text))
    }
}
