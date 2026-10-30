package dev.kampr.shared

import dev.kampr.shared.net.commitAttachmentId
import dev.kampr.shared.net.diffAttachmentId
import dev.kampr.shared.net.fileAttachmentId
import dev.kampr.shared.net.filePathOf
import dev.kampr.shared.net.lsAttachmentId
import dev.kampr.shared.net.pathOfAttachmentId
import dev.kampr.shared.net.repoAttachmentId
import dev.kampr.shared.net.showAttachmentId
import kotlin.io.encoding.Base64
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

// The route reads these, and the two below are the protocol's own worked examples
// (docs/04-wire-protocol.md, "A second id form"). An id that decodes to anything else is a 404
// with no way to tell why, so the encoding is the whole of what this has to get right.
class AttachmentIdTest {
    @Test
    fun aFileIdIsTheProtocolsOwnWorkedExample() {
        assertEquals("ZmlsZR8vdmFyL2xpYi9rYW1wci9zaG90LnBuZw", fileAttachmentId("/var/lib/kampr/shot.png"))
    }

    // A leading `~/` is resolved against the *node's* home, so it travels as itself.
    @Test
    fun aHomeAnchoredPathTravelsAsItself() {
        assertEquals("ZmlsZR9-L3Nob3QucG5n", fileAttachmentId("~/shot.png"))
    }

    @Test
    fun aDiffIdIsTheSamePathUnderTheOtherTag() {
        val id = diffAttachmentId("/var/lib/kampr/shot.png")
        assertEquals("/var/lib/kampr/shot.png", pathOfAttachmentId(id))
        assertNotEquals(fileAttachmentId("/var/lib/kampr/shot.png"), id, "both tags minted the same id")
        assertTrue('=' !in id, "base64url with no padding is one path segment; $id is not")
    }

    @Test
    fun aPathComesBackOutOfAnIdThisClientMinted() {
        assertEquals("~/dev/x/plot.png", pathOfAttachmentId(fileAttachmentId("~/dev/x/plot.png")))
    }

    // A record id is five fields and names a record in a transcript. There is no path in one, and
    // answering with the first field would hand the file viewer a path an agent never wrote.
    @Test
    fun aRecordIdCarriesNoPath() {
        assertNull(pathOfAttachmentId("Y2xhdWRlHwBwcm9qZWN0cy94Lmpzb25sHzEyHzAfNTA"))
        assertNull(pathOfAttachmentId("not base64 at all !!"))
        assertNull(pathOfAttachmentId(""))
    }

    // Absolute or anchored at the home, because those are the only two forms the route resolves:
    // a relative path is refused there rather than guessed at against a working directory the
    // request has no say in.
    @Test
    fun onlyAPathTheRouteCouldResolveIsOfferedAsOne() {
        assertEquals("/home/u/notes.md", filePathOf("/home/u/notes.md"))
        assertEquals("~/notes.md", filePathOf(" ~/notes.md "))
        assertNull(filePathOf("list panes"))
        assertNull(filePathOf("src/main.rs"))
        assertNull(filePathOf("~notes.md"))
        assertNull(filePathOf("/home/u/dev/"), "a directory is a 404 and never a file")
        assertNull(filePathOf(null))
    }

    @Test
    fun theBrowsingFormsAreTheirTagAndFieldsJoinedByTheSeparator() {
        val cases = mapOf(
            lsAttachmentId("~/dev") to "ls\u001F~/dev",
            repoAttachmentId("/home/u/dev/kampr") to "repo\u001F/home/u/dev/kampr",
            showAttachmentId("/home/u/dev/kampr/a.kt", "HEAD~1") to "show\u001F/home/u/dev/kampr/a.kt\u001FHEAD~1",
            commitAttachmentId("/home/u/dev/kampr", "8a50ea5") to "commit\u001F/home/u/dev/kampr\u001F8a50ea5",
        )
        for ((id, text) in cases) {
            assertTrue('=' !in id && '/' !in id && '+' !in id, "not base64url-no-pad: $id")
            assertEquals(text, unmint(id))
        }
    }

    // The file viewer reads a path out of an id to ask for the diff beside it; a listing or a commit
    // is not a file, and offering one a diff would be a 404 dressed as a feature.
    @Test
    fun onlyTheFileAndDiffFormsAnswerWithAPath() {
        assertNull(pathOfAttachmentId(lsAttachmentId("~/dev")))
        assertNull(pathOfAttachmentId(repoAttachmentId("~/dev")))
        assertNull(pathOfAttachmentId(showAttachmentId("~/dev/a.kt", "HEAD")))
        assertNull(pathOfAttachmentId(commitAttachmentId("~/dev", "abcd")))
    }

    private fun unmint(id: String): String = Base64.UrlSafe.decode(id + "=".repeat((4 - id.length % 4) % 4)).decodeToString()
}
