package dev.kampr.conversation

import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import dev.kampr.conversation.md.markPaths
import dev.kampr.conversation.md.pathLink
import dev.kampr.conversation.md.pathOfLink
import dev.kampr.shared.model.resolvePath
import kotlin.test.Test
import kotlin.test.assertEquals

class PathLinkTest {
    private val styles = testInlineStyles()

    private fun paths(source: String) = linkUrls(inlineText(source, styles)).mapNotNull(::pathOfLink)

    @Test
    fun aPathAnAgentNamesInProseIsATarget() {
        val cases = listOf(
            "I changed /home/u/dev/kampr/Makefile today." to listOf("/home/u/dev/kampr/Makefile"),
            "see ~/notes/todo.md, then stop" to listOf("~/notes/todo.md"),
            "edited client/shared/src/Native.kt and ./build.sh" to listOf("client/shared/src/Native.kt", "./build.sh"),
            "(in ../docs/04-wire-protocol.md)" to listOf("../docs/04-wire-protocol.md"),
            "the failure is at crates/kampr-node/src/http.rs:646:5." to listOf("crates/kampr-node/src/http.rs"),
        )
        for ((source, expected) in cases) assertEquals(expected, paths(source), source)
    }

    @Test
    fun aCodeSpanThatIsAPathIsATargetAndKeepsItsText() {
        val text = inlineText("open `Native.kt` and `client/shared/build.gradle.kts`", styles)
        assertEquals(listOf("Native.kt", "client/shared/build.gradle.kts"), linkUrls(text).mapNotNull(::pathOfLink))
        assertEquals("open Native.kt and client/shared/build.gradle.kts", text.text)
    }

    @Test
    fun proseThatMerelyHasASlashOrADotIsNotAPath() {
        val prose = listOf(
            "and/or TCP/IP 1/2 either/or",
            "version 0.1.127 shipped, e.g. today",
            "the kampr_core::pane module",
            "https://kampr.dev/docs/a.md is a url",
            "a // comment and a / alone",
        )
        for (source in prose) assertEquals(emptyList(), paths(source), source)
        assertEquals(emptyList(), paths("`foo.bar()` and `v1.2.3` and `x = a/b`"))
    }

    @Test
    fun aUrlIsStillAUrlBesideAPath() {
        val text = inlineText("read https://kampr.dev/a and /etc/hosts", styles)
        assertEquals(listOf("https://kampr.dev/a", pathLink("/etc/hosts")), linkUrls(text))
    }

    @Test
    fun plainTextMarksPathsTheSameWay() {
        val out = AnnotatedString("src/main.rs:12: error\n  --> crates/kampr-node/src/fs.rs:3:1\nok").markPaths(SpanStyle())
        assertEquals(listOf("src/main.rs", "crates/kampr-node/src/fs.rs"), linkUrls(out).mapNotNull(::pathOfLink))
    }

    @Test
    fun aRelativePathResolvesAgainstThePanesDirectory() {
        val cases = listOf(
            Triple("client/x.kt", "/home/u/dev/kampr", "/home/u/dev/kampr/client/x.kt"),
            Triple("./build.sh", "/home/u/dev/kampr/", "/home/u/dev/kampr/build.sh"),
            Triple("../docs/a.md", "/home/u/dev/kampr/client", "/home/u/dev/kampr/docs/a.md"),
            Triple("/etc/hosts", "/home/u", "/etc/hosts"),
            Triple("~/notes.md", "/home/u/dev", "~/notes.md"),
            Triple("x.kt", null, "~/x.kt"),
            Triple("../../../../x", "/a", "/x"),
        )
        for ((path, cwd, expected) in cases) assertEquals(expected, resolvePath(path, cwd), "$path in $cwd")
    }
}
