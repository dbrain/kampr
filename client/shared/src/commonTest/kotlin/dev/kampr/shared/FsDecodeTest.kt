package dev.kampr.shared

import dev.kampr.shared.net.FsKind
import dev.kampr.shared.net.anchorPane
import dev.kampr.shared.net.decodeListing
import dev.kampr.shared.net.decodeRepo
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

private const val LISTING = """{
  "path": "/home/u/dev/kampr", "parent": "/home/u/dev", "home": "/home/u",
  "entries": [
    {"name": "src", "kind": "dir", "bytes": null, "mtime": 1730000000, "git": "M"},
    {"name": "README.md", "kind": "file", "bytes": 4123, "mtime": 1730000000, "git": null},
    {"name": "link", "kind": "link", "target_kind": "dir", "bytes": null, "mtime": 1730000000, "git": null},
    {"name": "dangling", "kind": "link", "target_kind": null, "bytes": null, "mtime": 1730000000, "git": null},
    {"name": "fifo", "kind": "other", "bytes": 0, "mtime": 1730000000, "git": null, "later": true}
  ],
  "truncated": false,
  "repo": {"root": "/home/u/dev/kampr", "branch": "main", "head": "8a50ea5", "upstream": "origin/main",
           "ahead": 0, "behind": 2, "changed": 3}
}"""

private const val ROOT = """{"path": "/", "parent": null, "home": "/root", "entries": [], "truncated": true, "repo": null}"""

private const val REPO = """{
  "root": "/home/u/dev/kampr", "branch": null, "head": "8a50ea5", "upstream": null,
  "ahead": 0, "behind": 2,
  "changes": [
    {"path": "client/x.kt", "abs": "/home/u/dev/kampr/client/x.kt", "staged": "M", "unstaged": " ", "orig": null},
    {"path": "b.kt", "abs": "/home/u/dev/kampr/b.kt", "staged": "R", "unstaged": "M", "orig": "a.kt"}
  ],
  "log": [
    {"sha": "8a50ea5aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "short": "8a50ea5", "subject": "kobup: 0.1.127", "author": "Danny", "when": 1730000000}
  ]
}"""

class FsDecodeTest {
    @Test
    fun aListingDecodesAsTheNodeWritesIt() {
        val listing = assertNotNull(decodeListing(LISTING.encodeToByteArray()))
        assertEquals("/home/u/dev/kampr", listing.path)
        assertEquals("/home/u/dev", listing.parent)
        assertEquals(listOf(FsKind.Dir, FsKind.File, FsKind.Link, FsKind.Link, FsKind.Other), listing.entries.map { it.kind })
        assertEquals(4123L, listing.entries[1].bytes)
        assertEquals("M", listing.entries[0].git)
        assertTrue(listing.entries[2].isDir, "a link to a directory is walked into like one")
        assertFalse(listing.entries[3].isDir, "a dangling link has nowhere to go")
        val repo = assertNotNull(listing.repo)
        assertEquals(2, repo.behind)
        assertEquals(3, repo.changed)
    }

    @Test
    fun theRootHasNoParentAndAPlainDirectoryNoRepo() {
        val root = assertNotNull(decodeListing(ROOT.encodeToByteArray()))
        assertNull(root.parent)
        assertNull(root.repo)
        assertTrue(root.truncated)
    }

    @Test
    fun aRepoSummaryDecodesWithItsRenameAndDetachedHead() {
        val repo = assertNotNull(decodeRepo(REPO.encodeToByteArray()))
        assertNull(repo.branch)
        assertEquals("a.kt", repo.changes[1].orig)
        assertEquals("Danny", repo.log.single().author)
        assertEquals(1730000000L, repo.log.single().`when`)
    }

    @Test
    fun bytesThatAreNotTheShapeAreNullNotAThrow() {
        assertNull(decodeListing("no such attachment".encodeToByteArray()))
        assertNull(decodeRepo(LISTING.encodeToByteArray()))
        assertNull(decodeListing(byteArrayOf(-1, -2, 0)))
    }

    @Test
    fun aNodeWithNoPaneToHandIsAnchoredAtItself() {
        assertEquals("01JHUB/_", anchorPane("01JHUB"))
    }
}
