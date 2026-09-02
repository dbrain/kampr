package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.ImageComposeScene
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import dev.kampr.conversation.md.Markdown
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import org.jetbrains.skia.Bitmap
import org.jetbrains.skia.ColorAlphaType
import org.jetbrains.skia.ImageInfo
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotEquals
import kotlin.test.assertTrue

// Prose the conversation actually renders, carrying the symbols the corpus says it carries.
private const val PROSE = "✅ the width inference landed ● and ⎿ the tree held ⇒ done 🎧"
private const val PLAIN = "the width inference landed and the tree held, done."

private fun drawn(source: String, tokens: KamprTokens): Int {
    val scene = ImageComposeScene(width = 420, height = 260, density = Density(2f)) {
        CompositionLocalProvider(LocalTokens provides tokens) {
            Box(Modifier.fillMaxSize()) { Markdown(source, "", Modifier.fillMaxSize()) }
        }
    }
    val image = scene.render()
    val info = ImageInfo.makeN32(image.width, image.height, ColorAlphaType.UNPREMUL)
    val bitmap = Bitmap().also { it.allocPixels(info) }
    check(image.readPixels(bitmap)) { "the scene rendered no pixels" }
    val bytes = bitmap.readPixels()!!
    scene.close()
    // The ground is opaque, so ink is whatever is not the ground. Counted rather than compared as
    // an image: #271 already learned that a control image here fails on antialiasing alone.
    val ground = bytes.copyOfRange(0, 4).toList()
    return (bytes.indices step 4).count { bytes.copyOfRange(it, it + 4).toList() != ground }
}

// The seam, end to end through the view that renders a message. `routed = false` is the same
// tokens with the gap tables emptied — which is exactly what shipped before this — so a difference
// between the two is the routing doing something, and no difference is the routing being unwired.
//
// It cannot be asserted as "no tofu": this JVM has system fonts behind Skia and a browser has
// none, so the prose face draws *something* here either way. What can be asserted is that the
// routed rendering is the one the terminal face produces and the unrouted one is not.
class ProseGlyphTest {
    private fun tokens(routed: Boolean) =
        tokensFor(SoftTheme, TypeScale.Phone, Ground.Dark, routed = routed)

    @Test
    fun aMessageCarryingSymbolsTheProseFaceCannotDrawIsRenderedDifferentlyOnceTheyAreRouted() {
        val unrouted = drawn(PROSE, tokens(routed = false))
        val routed = drawn(PROSE, tokens(routed = true))
        assertTrue(unrouted > 0 && routed > 0, "nothing was drawn at all: $unrouted / $routed")
        assertNotEquals(
            unrouted,
            routed,
            "routing changed nothing about a paragraph carrying ✅ ● ⎿ ⇒ 🎧 — the seam is not " +
                "wired, and in a browser every one of them is tofu",
        )
    }

    // And it touches nothing else. A paragraph of ordinary prose has no character the face is
    // missing, so the two renderings are identical to the pixel — which is the half that says this
    // is a fix and not a change of typeface.
    @Test
    fun ordinaryProseIsRenderedIdenticallyWhetherTheTableIsThereOrNot() {
        assertEquals(
            drawn(PLAIN, tokens(routed = false)),
            drawn(PLAIN, tokens(routed = true)),
            "routing moved a glyph the prose face could already draw",
        )
    }
}
