package dev.kampr.shared

import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.ThemeMode
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.ui.AppearanceScreen
import java.io.File
import kotlin.test.Test

// The widths the count changes at, which is the whole of what the grid does now. 600 dp is what a
// 900 dp desktop window actually hands this screen once the sidebar has had its share, and it is
// the one the report came off.
private val BODIES = listOf(411, 600, 720, 1000, 1440)

class AppearanceArtboardTest {
    @Test
    fun theThemeGridRendersAtEveryWidthItsColumnCountChangesAt() {
        for (width in BODIES) {
            render(
                width.dp, 900.dp, themeOf("soft"), TypeScale.Desk,
                File("build/artboards/appearance-$width.png"),
                ground = Ground.Dark,
                density = Density(1.5f),
            ) {
                AppearanceScreen(themeOf("soft").id, ThemeMode.Dark, {}, {}, {})
            }
        }
    }

    // The one the alignment question is about: at 3440 the grid is centred and the title bar is
    // not, and whether that reads as a mistake can only be answered by looking at it.
    @Test
    fun theThemeGridRendersOnAnUltrawide() {
        render(
            3440.dp, 900.dp, themeOf("soft"), TypeScale.Desk,
            File("build/artboards/appearance-3440.png"),
            ground = Ground.Dark,
            density = Density(1f),
        ) {
            AppearanceScreen(themeOf("soft").id, ThemeMode.Dark, {}, {}, {})
        }
    }
}
