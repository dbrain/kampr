package dev.kampr.mosaic

import java.io.File
import kotlin.test.Test
import kotlin.test.assertTrue
import kotlin.test.fail

// The same rule shared and the renderer both enforce on themselves: phosphor / warm /
// brutalist stay one attribute away, and a literal anywhere outside the token layer breaks that
// quietly enough that only a screenshot in a second theme would catch it.
class TokenLayerTest {
    private val offenders = listOf(
        Regex("""Color\(0x[0-9A-Fa-f]+\)""") to "colour literal",
        Regex("""Color\.(White|Black|Red|Green|Blue|Gray|Yellow|Cyan|Magenta|Transparent)""") to "named colour",
        Regex("""FontFamily\(""") to "font family",
        Regex("""FontFamily\.(Monospace|SansSerif|Serif|Cursive|Default)""") to "font family",
        Regex("""RoundedCornerShape\(\s*\d""") to "radius literal",
    )

    private fun sourceRoot(): File {
        var dir = File(".").absoluteFile
        repeat(4) {
            val candidate = File(dir, "src/commonMain/kotlin/dev/kampr/mosaic")
            if (candidate.isDirectory) return candidate
            dir = dir.parentFile ?: return@repeat
        }
        fail("could not locate mosaic common sources from ${File(".").absolutePath}")
    }

    @Test
    fun noColourFontOrRadiusLiteralInTheMosaic() {
        val problems = mutableListOf<String>()
        sourceRoot().walkTopDown()
            .filter { it.isFile && it.extension == "kt" }
            .forEach { file ->
                file.readLines().forEachIndexed { index, line ->
                    if (line.trimStart().startsWith("//")) return@forEachIndexed
                    for ((pattern, what) in offenders) {
                        if (pattern.containsMatchIn(line)) {
                            problems += "${file.name}:${index + 1} $what -> ${line.trim()}"
                        }
                    }
                }
            }
        assertTrue(
            problems.isEmpty(),
            "these must resolve through dev.kampr.shared.theme instead:\n" + problems.joinToString("\n"),
        )
    }
}
