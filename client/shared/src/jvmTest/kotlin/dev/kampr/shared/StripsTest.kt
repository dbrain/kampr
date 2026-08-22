package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.ui.ErrorStrip
import dev.kampr.shared.ui.NoteStrip
import dev.kampr.shared.ui.SafeArea
import kotlin.test.Test
import kotlin.test.assertTrue

private const val SHORT = "Passkey enrolled. This device now signs in with it."

// What the self-describing failure path produces: an explanation and the lines to paste. Three
// lines of it were on screen and the rest was not.
private const val LONG =
    "This node names dev.kampr.app but not the certificate this build is signed with. " +
        "Add it to [android] fingerprints in its config.toml:\n\n" +
        "\"A0:8A:21:84:46:AA:2B:99:08:5C:67:0B:5A:9B:70:32:5E:05:F9:27:CC:DD:12:17:E7:94:63:13:C7:7F:C6:18\""

// The strips float over the whole window rather than inside a screen, so nothing else pays their
// insets for them — and the operator's report was a green strip they could not read at all,
// drawn under the status bar and behind the punch-hole.
@OptIn(ExperimentalTestApi::class)
class StripsTest {
    @Test
    fun aStripClearsTheStatusBarAndTheCutout() {
        for (bars in listOf(BARS) + SIDE_BARS) {
            runComposeUiTest {
                setContent {
                    Bars(bars) {
                        Box(Modifier.fillMaxSize()) {
                            NoteStrip(SHORT, {})
                        }
                    }
                }
                val screen = onRoot().getUnclippedBoundsInRoot()
                val strip = onNodeWithContentDescription(SHORT, substring = true).getUnclippedBoundsInRoot()
                assertTrue(
                    strip.top >= screen.top + bars.top,
                    "$bars: the strip starts at ${strip.top}, inside the ${bars.top} status bar",
                )
                assertTrue(
                    strip.left >= screen.left + bars.left && strip.right <= screen.right - bars.right,
                    "$bars: the strip spans ${strip.left}..${strip.right} of ${screen.right}",
                )
            }
        }
    }

    @Test
    fun anErrorStripClearsThemToo() {
        runComposeUiTest {
            setContent {
                Bars(BARS) {
                    Box(Modifier.fillMaxSize()) { ErrorStrip(SHORT, "passkey", {}) }
                }
            }
            val screen = onRoot().getUnclippedBoundsInRoot()
            val strip = onNodeWithContentDescription(SHORT, substring = true).getUnclippedBoundsInRoot()
            assertTrue(
                strip.top >= screen.top + BARS.top,
                "the error strip starts at ${strip.top}, inside the ${BARS.top} status bar",
            )
        }
    }

    // Measured against the strip's own line height rather than a number of pixels: one line and
    // two lines give the height of a line and the height of everything that is not one, and every
    // further line has to be there on top of that. A cap of three passed a one-line assertion and
    // still ate the half of the message that says what to paste.
    @Test
    fun aLongExplanationIsNotClippedToThreeLines() {
        val one = stripHeight("x")
        val line = stripHeight("x\nx") - one
        val eight = stripHeight(List(8) { "x" }.joinToString("\n"))
        assertTrue(
            eight >= one + line * 7,
            "eight lines came to $eight, and one line plus seven more would be ${one + line * 7}",
        )
        assertTrue(
            stripHeight(LONG) > one + line * 3,
            "the config lines to paste do not fit: ${stripHeight(LONG)} against a three-line ${one + line * 3}",
        )
    }
}

@OptIn(ExperimentalTestApi::class)
private fun stripHeight(message: String): Dp {
    var height = 0.dp
    runComposeUiTest {
        setContent { Bars(BARS) { Box(Modifier.fillMaxSize()) { NoteStrip(message, {}) } } }
        height = onNodeWithContentDescription(message, substring = true)
            .getUnclippedBoundsInRoot()
            .let { it.bottom - it.top }
    }
    return height
}
