package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.ImageComposeScene
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.platform.Font
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import dev.kampr.shared.theme.FamilyId
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.ThemeSpec
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.on
import dev.kampr.shared.theme.typography
import java.io.File

// Compose resources resolve fonts asynchronously and a headless render never waits for that, so
// the artboards load the same .ttf files straight off disk.
internal fun family(id: FamilyId): FontFamily {
    val dir = File("src/commonMain/composeResources/font")
    fun face(name: String, weight: FontWeight) = Font(name, File(dir, "$name.ttf").readBytes(), weight)
    return when (id) {
        FamilyId.Manrope -> FontFamily(
            face("manrope_400", FontWeight.W400), face("manrope_500", FontWeight.W500),
            face("manrope_600", FontWeight.W600), face("manrope_700", FontWeight.W700),
            face("manrope_800", FontWeight.W800),
        )
        FamilyId.IbmPlexMono -> FontFamily(
            face("ibmplexmono_400", FontWeight.W400), face("ibmplexmono_500", FontWeight.W500),
            face("ibmplexmono_600", FontWeight.W600),
        )
        FamilyId.JetBrainsMono -> FontFamily(
            face("jetbrainsmono_400", FontWeight.W400), face("jetbrainsmono_500", FontWeight.W500),
            face("jetbrainsmono_700", FontWeight.W700),
        )
        FamilyId.InstrumentSans -> FontFamily(
            face("instrumentsans_400", FontWeight.W400), face("instrumentsans_500", FontWeight.W500),
            face("instrumentsans_600", FontWeight.W600),
        )
        FamilyId.Archivo -> FontFamily(
            face("archivo_500", FontWeight.W500), face("archivo_700", FontWeight.W700),
            face("archivo_900", FontWeight.W900),
        )
    }
}

internal fun tokensFor(spec: ThemeSpec, scale: TypeScale, ground: Ground): KamprTokens {
    val grounded = spec.on(ground)
    val fonts = KamprFonts(family(grounded.ui), family(grounded.mono), family(FamilyId.JetBrainsMono))
    return KamprTokens(grounded, fonts, typography(fonts, grounded.label, scale))
}

// `density` is the whole point on a phone artboard: 1080x2400 is 360 dp at 3x and 411 dp at 2.625x,
// and the header defect only exists at one of them.
internal fun render(
    width: Dp,
    height: Dp,
    spec: ThemeSpec,
    scale: TypeScale,
    file: File,
    ground: Ground = Ground.Dark,
    density: Density = Density(2f),
    content: @Composable () -> Unit,
): org.jetbrains.skia.Image {
    val scene = ImageComposeScene(
        width = with(density) { width.roundToPx() },
        height = with(density) { height.roundToPx() },
        density = density,
    ) {
        CompositionLocalProvider(LocalTokens provides tokensFor(spec, scale, ground)) {
            Box(Modifier.fillMaxSize()) { content() }
        }
    }
    return try {
        scene.render()
        val image = scene.render()
        file.parentFile.mkdirs()
        file.writeBytes(requireNotNull(image.encodeToData()).bytes)
        image
    } finally {
        scene.close()
    }
}
