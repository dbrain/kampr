package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.on
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.BottomNav
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.SafeArea
import dev.kampr.shared.ui.Tab
import kotlin.test.Test
import kotlin.test.assertTrue

// A gesture handle's worth of system furniture, and a status bar's. Real values on the API 37
// AVD this was found on: 1080×2400 at 420 dpi.
private val BARS = SafeArea(top = 32.dp, bottom = 46.dp)

private fun tokens() = themeOf("soft").on(Ground.Dark).let { spec ->
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    KamprTokens(spec, fonts, typography(fonts, spec.label, TypeScale.Phone))
}

@Composable
private fun Bars(content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalTokens provides tokens(), LocalSafeArea provides BARS, content = content)
}

// Every layout assertion in this suite measures Compose's own semantics tree, which knows nothing
// about what SystemUI paints on top of it — so a full suite passed while the gesture handle sat on
// the word "Pane". `LocalSafeArea` exists so a test can put the bars back.
@OptIn(ExperimentalTestApi::class)
class SafeAreaTest {
    @Test
    fun theBottomNavigationClearsTheGestureHandle() = runComposeUiTest {
        setContent {
            Bars {
                Column(Modifier.fillMaxSize()) {
                    Box(Modifier.weight(1f))
                    BottomNav(Tab.Herd, {})
                }
            }
        }
        // Against the window rather than a number: a fixed height taller than the test window is
        // how this assertion passed while the defect was on screen.
        val screen = onRoot().getUnclippedBoundsInRoot()
        for (tab in listOf("Herd tab", "Pane tab", "Nodes tab")) {
            val bounds = onNodeWithContentDescription(tab).getUnclippedBoundsInRoot()
            assertTrue(
                bounds.bottom <= screen.bottom - BARS.bottom,
                "$tab reaches ${bounds.bottom} of ${screen.bottom}, inside the ${BARS.bottom} the " +
                    "system draws its gesture handle in",
            )
        }
    }
}
