package dev.kampr.conversation.files

import dev.kampr.shared.net.FsEntry
import dev.kampr.shared.net.FsKind
import dev.kampr.shared.net.RepoChange
import kotlin.test.Test
import kotlin.test.assertEquals

private const val COMMIT = """commit 8a50ea5aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
Author:     Danny <d@x>
AuthorDate: Thu Oct 29 10:00:00 2026 +0000

    kobup: 0.1.127

 a.kt | 2 +-
 b.png | Bin 0 -> 12 bytes
 gone.kt | 1 -

diff --git a/src/a.kt b/src/a.kt
index 1111111..2222222 100644
--- a/src/a.kt
+++ b/src/a.kt
@@ -1 +1 @@
-old
+new
diff --git a/b.png b/b.png
new file mode 100644
index 0000000..3333333
Binary files /dev/null and b/b.png differ
diff --git a/gone.kt b/gone.kt
deleted file mode 100644
--- a/gone.kt
+++ /dev/null
@@ -1 +0,0 @@
-bye
"""

private fun entry(name: String, kind: FsKind = FsKind.File, target: FsKind? = null) = FsEntry(name, kind, target)

private fun change(path: String, x: String, y: String) = RepoChange(path, "/r/$path", x, y)

class BrowseTest {
    @Test
    fun crumbsAnchorAtHomeWhenThePathIsUnderIt() {
        assertEquals(
            listOf(Crumb("~", "/home/u"), Crumb("dev", "/home/u/dev"), Crumb("kampr", "/home/u/dev/kampr")),
            crumbs("/home/u/dev/kampr", "/home/u"),
        )
        assertEquals(listOf(Crumb("/", "/"), Crumb("etc", "/etc")), crumbs("/etc", "/home/u"))
        assertEquals(listOf(Crumb("/", "/"), Crumb("home", "/home"), Crumb("u2", "/home/u2")), crumbs("/home/u2", "/home/u"))
        assertEquals(listOf(Crumb("/", "/")), crumbs("/", "/"))
    }

    @Test
    fun childPathsDoNotDoubleTheRootSlash() {
        assertEquals("/etc", childPath("/", "etc"))
        assertEquals("/home/u/a", childPath("/home/u", "a"))
    }

    @Test
    fun visibleEntriesHideDotfilesFilterAndPutDirectoriesFirst() {
        val entries = listOf(
            entry("zeta.txt"),
            entry(".git", FsKind.Dir),
            entry("Beta", FsKind.Dir),
            entry("alpha", FsKind.Link, FsKind.Dir),
            entry(".env"),
            entry("Apple.md"),
        )
        assertEquals(listOf("alpha", "Beta", "Apple.md", "zeta.txt"), visibleEntries(entries, "", false).map { it.name })
        assertEquals(listOf(".git", "alpha", "Beta", ".env", "Apple.md", "zeta.txt"), visibleEntries(entries, "", true).map { it.name })
        assertEquals(listOf("alpha", "Apple.md"), visibleEntries(entries, " P ", false).map { it.name })
        assertEquals(listOf(".env"), visibleEntries(entries, ".en", false).map { it.name })
    }

    @Test
    fun sizesReadAsAPersonWouldSayThem() {
        assertEquals("0 B", formatBytes(0))
        assertEquals("1023 B", formatBytes(1023))
        assertEquals("4.0 KB", formatBytes(4123))
        assertEquals("12 MB", formatBytes(12L * 1024 * 1024 + 5))
    }

    @Test
    fun changesGroupByWhereGitHoldsThem() {
        val groups = groupChanges(
            listOf(change("a", "M", " "), change("b", "M", "M"), change("c", " ", "D"), change("d", "?", "?"), change("e", "U", "U")),
        )
        assertEquals(listOf("a" to "M", "b" to "M", "e" to "U"), groups.staged.map { it.change.path to it.letter })
        assertEquals(listOf("b" to "M", "c" to "D", "e" to "U"), groups.unstaged.map { it.change.path to it.letter })
        assertEquals(listOf("d" to "?"), groups.untracked.map { it.change.path to it.letter })
    }

    @Test
    fun aCommitSplitsIntoItsHeaderAndOnePatchPerFile() {
        val parts = splitCommit(COMMIT)
        assertEquals(true, parts.header.startsWith("commit 8a50ea5"))
        assertEquals(true, parts.header.endsWith("gone.kt | 1 -"))
        assertEquals(listOf("src/a.kt", "b.png", "gone.kt"), parts.files.map { it.path })
        assertEquals("@@ -1 +1 @@\n-old\n+new", parts.files[0].text)
        assertEquals(true, parts.files[1].text.contains("Binary files"))
        assertEquals("@@ -1 +0,0 @@\n-bye", parts.files[2].text)
    }

    @Test
    fun aCommitWithNoPatchIsAllHeader() {
        assertEquals(CommitParts("commit abc\n\n    empty", emptyList()), splitCommit("commit abc\n\n    empty\n"))
    }
}
