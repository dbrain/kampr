package dev.kampr.shared

import java.io.File
import kotlin.test.Test
import kotlin.test.assertTrue
import kotlin.test.fail

// The sibling of TokenLayerTest, and it exists for the same reason: a control wired straight to
// `clickable` compiles, looks right, and is a button that announces itself as "button". Routing
// every one of them through `Modifier.action` is what makes a name compulsory rather than
// remembered, and the only way that survives contact with a hurry is a test.
class SemanticsLayerTest {
    private val bare = listOf(
        Regex("""\.clickable\(""") to "clickable",
        Regex("""detectTapGestures\(""") to "raw tap gesture",
    )

    // Where the two primitives are implemented, and the one place a full-screen scrim is built.
    private val allowed = setOf("Accessibility.kt")

    private fun clientRoot(): File {
        var dir = File(".").absoluteFile
        repeat(4) {
            if (File(dir, "shared/src/commonMain").isDirectory) return dir
            dir = dir.parentFile ?: return@repeat
        }
        fail("could not locate the client root from ${File(".").absolutePath}")
    }

    private fun surfaces(): List<File> = listOf("shared", "terminal", "conversation", "mosaic")
        .map { clientRoot().resolve("$it/src/commonMain") }
        .filter { it.isDirectory }
        .flatMap { it.walkTopDown().filter { file -> file.isFile && file.extension == "kt" } }

    @Test
    fun nothingIsClickableWithoutBeingNamed() {
        val problems = mutableListOf<String>()
        for (file in surfaces()) {
            if (file.name in allowed) continue
            val text = file.readText()
            val named = "gestureAction(" in text
            for ((pattern, what) in bare) {
                if (!pattern.containsMatchIn(text) || named) continue
                problems += "${file.name} reaches for $what without a semantics action beside it"
            }
        }
        assertTrue(
            problems.isEmpty(),
            "these must go through Modifier.action or Modifier.gestureAction:\n" + problems.joinToString("\n"),
        )
    }

    // prefers-reduced-motion is not a preference about taste. A vestibular disorder makes a
    // momentum fling and a blinking caret genuinely unpleasant, and the setting existing is worth
    // nothing if the next animation added forgets to ask.
    @Test
    fun everythingThatMovesAsksWhetherItShould() {
        val movers = listOf(
            Regex("animateScrollToItem\\(") to "an animated scroll",
            Regex("animateScrollBy\\(") to "an animated scroll",
            Regex("\\bAnimatable\\(") to "an Animatable",
            Regex("animate\\w*AsState\\(") to "an animated state",
            Regex("CURSOR_BLINK_MS") to "a blinking caret",
        )
        val problems = mutableListOf<String>()
        for (file in surfaces()) {
            val text = file.readText()
            if ("LocalReduceMotion" in text) continue
            for ((pattern, what) in movers) {
                if (pattern.containsMatchIn(text)) {
                    problems += file.name + " drives " + what + " without reading LocalReduceMotion"
                }
            }
        }
        assertTrue(problems.isEmpty(), problems.joinToString("\n"))
    }

    // A screen with no headings is a screen a reader can only walk one control at a time.
    @Test
    fun everyScreenNamesItself() {
        val screens = clientRoot().resolve("shared/src/commonMain").walkTopDown()
            .filter { it.isFile && it.name.endsWith("Screen.kt") }
            .toList()
        assertTrue(screens.size >= 4, "expected to find the client's screens, found ${screens.size}")
        val silent = screens.filterNot { "asHeading()" in it.readText() }
        assertTrue(silent.isEmpty(), "no heading on: " + silent.joinToString { it.name })
    }
}
