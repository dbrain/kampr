package dev.kampr.terminal

import dev.kampr.terminal.render.TargetKind
import dev.kampr.terminal.render.detectTarget
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotEquals
import kotlin.test.assertNull

private fun at(line: String, inside: String): Int {
    val where = line.indexOf(inside)
    check(where >= 0) { "'$inside' is not in '$line'" }
    return where
}

// The terminal grid had one fetchable kind of target — a URL — and a path on it was a string to
// copy. A path an operator can already `cat` on their own machine is the second kind, and only
// where the pane wrote one nobody has to guess at.
class FileTargetTest {
    @Test
    fun anAbsolutePathOnTheGridIsAFileToOpen() {
        val line = "  Compiling kampr v0.1.30 (/home/dbrain/dev/kampr/src/main.rs)"
        val found = detectTarget(line, at(line, "/home/dbrain"))
        assertEquals(TargetKind.File, found?.kind)
        assertEquals("/home/dbrain/dev/kampr/src/main.rs", found?.text)
    }

    @Test
    fun aHomeRootedPathIsAFileToOpen() {
        val line = "wrote ~/.claude/settings.json"
        val found = detectTarget(line, at(line, "~/"))
        assertEquals(TargetKind.File, found?.kind)
        assertEquals("~/.claude/settings.json", found?.text)
    }

    // A compiler names the line and the column after the path, and the path is the part the node
    // can open. Keeping the location on it is a 404 for a file that is sitting right there.
    @Test
    fun theLineAndColumnAfterAPathAreNotPartOfIt() {
        val line = "error[E0433]: /home/dbrain/dev/kampr/crates/kampr-core/src/wire.rs:412:9"
        val found = detectTarget(line, at(line, "wire.rs"))
        assertEquals(TargetKind.File, found?.kind)
        assertEquals("/home/dbrain/dev/kampr/crates/kampr-core/src/wire.rs", found?.text)
    }

    @Test
    fun aSentenceEndingAfterAPathIsNotPartOfItEither() {
        val line = "I have written the whole thing to /tmp/kampr/notes.md."
        assertEquals("/tmp/kampr/notes.md", detectTarget(line, at(line, "/tmp"))?.text)
    }

    // The whole reason this is safe is that the operator can dispute nothing about it. A bare
    // `foo.rs` in prose is a guess about English; the existing reference target still copies it,
    // and it must never become a control that fetches a file.
    @Test
    fun aBarePathInProseIsNotAFile() {
        val line = "look at src/main.rs:12 for the call"
        val found = detectTarget(line, at(line, "src/"))
        assertNotEquals(TargetKind.File, found?.kind, "a relative path was offered as a file")
        assertEquals(TargetKind.Path, found?.kind)
    }

    @Test
    fun andOrIsNotAnAbsolutePath() {
        val line = "pass --all and/or --release"
        assertNull(detectTarget(line, at(line, "and/or")), "'/or' was read as a path")
    }

    // A directory is refused by the route with the same 404 as everything else, so offering one
    // is a button that can only ever fail.
    @Test
    fun aDirectoryIsNotAFile() {
        val line = "cd /home/dbrain/dev/kampr/"
        assertNull(detectTarget(line, at(line, "/home"))?.takeIf { it.kind == TargetKind.File })
    }

    // A URL is still a URL: `file://` and `https://` both hold a slash, and whichever wins has to
    // be the one the reader is looking at.
    @Test
    fun aUrlIsStillAUrl() {
        val line = "see https://herdr.dev/docs/panes for the rest"
        val found = detectTarget(line, at(line, "https"))
        assertEquals(TargetKind.Url, found?.kind)
        assertEquals("https://herdr.dev/docs/panes", found?.text)
    }

    @Test
    fun aPathInsideQuotesIsTheQuotedPath() {
        val line = """no such file: "/etc/kampr/node.toml""""
        assertEquals("/etc/kampr/node.toml", detectTarget(line, at(line, "/etc"))?.text)
    }

    @Test
    fun tappingWhitespaceIsNotTappingThePathBesideIt() {
        val line = "ls /home/dbrain/x.txt"
        assertNull(detectTarget(line, 2))
    }
}
