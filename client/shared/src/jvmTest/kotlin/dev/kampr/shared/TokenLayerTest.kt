package dev.kampr.shared

import java.io.File
import kotlin.test.Test
import kotlin.test.assertTrue
import kotlin.test.fail

// The whole point of the token layer is that phosphor / warm / brutalist stay one attribute
// away. A colour, font family or radius literal anywhere else silently breaks that, and it
// breaks quietly enough that only a screenshot in a second theme would catch it.
class TokenLayerTest {
    private val offenders = listOf(
        Regex("""Color\(0x[0-9A-Fa-f]+\)""") to "colour literal",
        Regex("""Color\.(White|Black|Red|Green|Blue|Gray|Yellow|Cyan|Magenta|Transparent)""") to "named colour",
        Regex("""FontFamily\(""") to "font family",
        Regex("""RoundedCornerShape\(\s*\d""") to "radius literal",
    )

    private fun sourceRoot(): File {
        var dir = File(".").absoluteFile
        repeat(4) {
            val candidate = File(dir, "src/commonMain/kotlin/dev/kampr/shared")
            if (candidate.isDirectory) return candidate
            dir = dir.parentFile ?: return@repeat
        }
        fail("could not locate shared common sources from ${File(".").absolutePath}")
    }

    @Test
    fun noColourFontOrRadiusLiteralOutsideTheTokenLayer() {
        val root = sourceRoot()
        val problems = mutableListOf<String>()
        root.walkTopDown()
            .filter { it.isFile && it.extension == "kt" }
            .filterNot { it.parentFile.name == "theme" }
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

    @Test
    fun everyThemeDefinesEveryTokenOnBothGrounds() {
        val root = sourceRoot().resolve("theme/Themes.kt").readText()
        val tokens = listOf("bg", "bar", "surface", "surface2", "raise", "line", "text", "dim",
            "mute", "accent", "accentHi", "onAccent", "accentSoft", "blocked", "blockedBg",
            "working", "idle", "done")
        for (family in listOf("SoftFamily", "PhosphorFamily", "WarmFamily", "BrutalistFamily")) {
            val block = root.substringAfter("val $family").substringBefore("\nval ")
            val dark = block.substringAfter("dark = Palette(").substringBefore("light = Palette(")
            val light = block.substringAfter("light = Palette(").substringBefore("radii =")
            for (token in tokens) {
                val pattern = Regex("""\b$token = """)
                assertTrue(pattern.containsMatchIn(dark), "$family dark is missing $token")
                assertTrue(pattern.containsMatchIn(light), "$family light is missing $token")
            }
        }
    }
}
