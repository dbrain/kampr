package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
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
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private fun tokens() = themeOf("soft").on(Ground.Dark).let { spec ->
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    KamprTokens(spec, fonts, typography(fonts, spec.label, TypeScale.Phone))
}

@Composable
private fun Chrome(content: @Composable () -> Unit) {
    CompositionLocalProvider(
        LocalTokens provides tokens(),
        LocalSafeArea provides SafeArea(top = 44.dp, bottom = 46.dp),
        content = content,
    )
}

// A tab whose purpose has to be guessed is a failed label, whatever it does internally. "Pane"
// meant the pane you last opened, which is not a peer of a list of everything, and "Nodes" led to
// the address, the pairing code, appearance and notifications — a settings screen wearing a
// machines label. The desktop sidebar had been calling the same destination Settings all along.
@OptIn(ExperimentalTestApi::class)
class NavLabelsTest {
    @Test
    fun theBarOffersOnlyLabelsThatSayWhereTheyGo() {
        runComposeUiTest {
            setContent { Chrome { Box(Modifier.fillMaxSize()) { BottomNav(Tab.Herd, {}) } } }
            for (gone in listOf("Pane tab", "Nodes tab")) {
                assertEquals(
                    0,
                    onAllNodesWithContentDescription(gone).fetchSemanticsNodes().size,
                    "$gone is still on the bar",
                )
            }
            for (kept in listOf("Herd tab", "Settings tab")) {
                assertTrue(
                    onAllNodesWithContentDescription(kept).fetchSemanticsNodes().isNotEmpty(),
                    "$kept is not on the bar",
                )
            }
        }
    }
}
